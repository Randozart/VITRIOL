#!/usr/bin/env python3
"""
VITRIOL Context Router & Memory Proxy

This shim sits between OpenCode (or any agent) and llama-server
(custom llama.cpp with VITRIOL), performing context rectification
and emulated memory retrieval to prevent OOM crashes and PCIe
prefill bottlenecks on legacy hardware.

OpenCode -> VITRIOL Shim (port 5010) -> llama-server (port 8279)

Implementation Plan v2.0 - Phase 2
"""

import json
import re
import os
import time
import subprocess
import requests
import logging
from flask import Flask, request, jsonify, Response, stream_with_context, Response, stream_with_context
from typing import List, Dict, Any, Optional
from dataclasses import dataclass
from datetime import datetime

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - VITRIOL - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

# Try to import FAISS for vector search
try:
    import faiss
    FAISS_AVAILABLE = True
except ImportError:
    FAISS_AVAILABLE = False
    logger.info("FAISS not available, will use keyword-based search")

# ── Emulated Memory Subsystem (config toggle) ──
MEMORY_MODE = os.environ.get('VITRIOL_MEMORY_MODE', 'off').lower() == 'on'
if MEMORY_MODE:
    try:
        import sys, os
        _parent = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        if _parent not in sys.path:
            sys.path.insert(0, _parent)
        from libvitriol import memory as emem
        logger.info("Emulated memory subsystem loaded (VITRIOL_MEMORY_MODE=on)")
    except ImportError as e:
        MEMORY_MODE = False
        logger.warning(f"Emulated memory import failed ({e}), falling back to standard mode")
else:
    logger.info("Emulated memory disabled (VITRIOL_MEMORY_MODE=off)")

app = Flask(__name__)

# Configuration - Implementation Plan v2.0
LLAMA_API_URL = os.environ.get('LLAMA_API_URL', "http://localhost:8279/v1/chat/completions")
LLAMA_STATUS_URL = os.environ.get('LLAMA_STATUS_URL', "http://localhost:8279/health")
MAX_CONTEXT_TOKENS = int(os.environ.get('MAX_CONTEXT_TOKENS', '7000'))
MAX_MESSAGES_TO_KEEP = int(os.environ.get('MAX_MESSAGES_TO_KEEP', '4'))
SHIM_PORT = int(os.environ.get('SHIM_PORT', '5010'))
MAX_TEMP = 85  # GPU thermal limit (°C)

# Context offloading strategy
CONTEXT_STRATEGY = os.environ.get('CONTEXT_STRATEGY', 'stream')

# Context streaming configuration
CONTEXT_STREAM_TOP_K = int(os.environ.get('CONTEXT_STREAM_TOP_K', '3'))
CONTEXT_STREAM_RELEVANCE_THRESHOLD = float(os.environ.get('CONTEXT_STREAM_RELEVANCE_THRESHOLD', '0.3'))

# Frozen prompt caching
FROZEN_PROMPT = os.environ.get('VITRIOL_FROZEN_PROMPT', 'off').lower() == 'on'
# Track the last frozen prefix hash to detect changes
_last_frozen_hash: Optional[int] = None

# Semantic search mode
SEMANTIC_MODE = os.environ.get('VITRIOL_SEMANTIC_MODE', 'off').lower() == 'on'
if SEMANTIC_MODE:
    logger.info("Semantic search enabled (VITRIOL_SEMANTIC_MODE=on)")


@dataclass
class RectificationStats:
    original_tokens: int
    rectified_tokens: int
    messages_dropped: int
    metadata_stripped: bool
    reduction_percent: float


def get_gpu_temp() -> int:
    """
    Guardrail 4: Thermal Polling via nvidia-smi
    Returns GPU temperature in Celsius, 0 on error
    """
    try:
        result = subprocess.run(
            ['nvidia-smi', '--id=0', '--query-gpu=temperature.gpu', '--format=csv,noheader,nounits'],
            capture_output=True,
            text=True,
            timeout=1
        )
        return int(result.stdout.strip().split('\n')[0])
    except Exception as e:
        logger.warning(f"Thermal poll failed: {e}")
        return 0


def archive_context_to_ssd(messages: List[Dict[str, Any]], archive_path: str = "/tmp/vitriol_context_archive.json") -> str:
    """
    Context Offloading Strategy 2: Archive old context to SSD
    Returns archive file path for potential retrieval
    """
    try:
        # Save to JSON
        with open(archive_path, 'w') as f:
            json.dump({
                'timestamp': datetime.now().isoformat(),
                'messages': messages,
                'token_count': sum(estimate_tokens(m.get('content', '')) for m in messages)
            }, f, indent=2)
        logger.info(f"Context archived to {archive_path} ({len(messages)} messages)")
        
        # Also add to vector store if available
        if FAISS_AVAILABLE:
            try:
                from .vector_store import get_vector_store
                chunks = chunk_messages_for_streaming(messages, chunk_size=5)
                vector_store = get_vector_store()
                vector_store.add_chunks(chunks)
                logger.info(f"Added {len(chunks)} chunks to vector store")
            except Exception as e:
                logger.warning(f"Vector store update failed: {e}")
        
        return archive_path
    except Exception as e:
        logger.error(f"Context archival failed: {e}")
        return ""


def retrieve_context_from_ssd(archive_path: str = "/tmp/vitriol_context_archive.json") -> List[Dict[str, Any]]:
    """
    Context Offloading Strategy 2: Retrieve archived context from SSD
    Returns messages list or empty list on error
    """
    if not os.path.exists(archive_path):
        logger.info("No archived context yet (first request or archive empty)")
        return []
    try:
        with open(archive_path, 'r') as f:
            data = json.load(f)
        logger.info(f"Context retrieved from {archive_path}")
        return data.get('messages', [])
    except Exception as e:
        logger.warning(f"Context retrieval failed: {e}")
        return []


def chunk_messages_for_streaming(messages: List[Dict[str, Any]], chunk_size: int = 5) -> List[Dict[str, Any]]:
    """
    Context Streaming: Split messages into overlapping chunks for better retrieval
    Each chunk contains context from neighboring messages
    """
    chunks = []
    for i in range(0, len(messages), chunk_size):
        chunk = messages[i:i + chunk_size * 2]  # Overlapping chunks
        if chunk:
            chunks.append({
                'id': f'chunk_{i}',
                'messages': chunk,
                'text': ' '.join([m.get('content', '') for m in chunk if m.get('content')]),
                'start_idx': i,
                'end_idx': i + len(chunk)
            })
    return chunks


def compute_relevance_score(query: str, chunk_text: str) -> float:
    """
    Context Streaming: Simple relevance scoring using keyword overlap
    Phase 2: Replace with embedding-based similarity (e.g., sentence-transformers)
    """
    query_words = set(query.lower().split())
    chunk_words = set(chunk_text.lower().split())
    
    if not query_words or not chunk_words:
        return 0.0
    
    # Jaccard similarity
    intersection = query_words & chunk_words
    union = query_words | chunk_words
    return len(intersection) / len(union) if union else 0.0


def stream_relevant_context(
    current_query: str,
    archive_path: str = "/tmp/vitriol_context_archive.json",
    top_k: int = CONTEXT_STREAM_TOP_K,
    threshold: float = CONTEXT_STREAM_RELEVANCE_THRESHOLD
) -> List[Dict[str, Any]]:
    """
    Context Streaming Strategy: Retrieve and inject relevant context from SSD
    
    Uses vector search (FAISS) for semantic similarity matching.
    Falls back to keyword-based search if FAISS unavailable.
    
    1. Load archived chunks
    2. Score each chunk by relevance to current query (vector similarity)
    3. Return top-K most relevant chunks
    4. Inject into conversation as "system context"
    """
    try:
        # Try vector-based search first
        if FAISS_AVAILABLE:
            from .vector_store import get_vector_store
            
            vector_store = get_vector_store()
            results = vector_store.search(current_query, top_k=top_k, threshold=threshold)
            
            if results:
                logger.info(f"Vector search found {len(results)} relevant chunks (scores: {[r['score'] for r in results]})")
                streamed_messages = []
                for result in results:
                    streamed_messages.extend(result.get('messages', []))
                return streamed_messages
        
        # Fallback to keyword-based search
        logger.info("Falling back to keyword-based context retrieval")
        return stream_relevant_context_keyword(current_query, archive_path, top_k, threshold)
        
    except Exception as e:
        logger.error(f"Context streaming failed: {e}")
        return []


def stream_relevant_context_keyword(
    current_query: str,
    archive_path: str = "/tmp/vitriol_context_archive.json",
    top_k: int = CONTEXT_STREAM_TOP_K,
    threshold: float = CONTEXT_STREAM_RELEVANCE_THRESHOLD
) -> List[Dict[str, Any]]:
    """
    Keyword-based context streaming (fallback when FAISS unavailable)
    """
    try:
        # Load archived context
        archived_messages = retrieve_context_from_ssd(archive_path)
        if not archived_messages:
            logger.info("No archived context to stream")
            return []
        
        # Chunk the archived context
        chunks = chunk_messages_for_streaming(archived_messages, chunk_size=5)
        logger.info(f"Split context into {len(chunks)} chunks")
        
        # Score each chunk by relevance
        scored_chunks = []
        for chunk in chunks:
            score = compute_relevance_score(current_query, chunk['text'])
            if score >= threshold:
                scored_chunks.append((score, chunk))
        
        # Sort by relevance and take top-K
        scored_chunks.sort(key=lambda x: x[0], reverse=True)
        top_chunks = scored_chunks[:top_k]
        
        logger.info(f"Streaming {len(top_chunks)} relevant context chunks (scores: {[s for s, _ in top_chunks]})")
        
        # Convert chunks back to messages
        streamed_messages = []
        for score, chunk in top_chunks:
            streamed_messages.extend(chunk['messages'])
        
        return streamed_messages
        
    except Exception as e:
        logger.error(f"Keyword context streaming failed: {e}")
        return []


def estimate_tokens(text: str) -> int:
    """Rough token estimation (4 chars ≈ 1 token for English)"""
    return len(text) // 4


def sublimate_content(content) -> str:
    """
    Sublimation (Guardrail 2): Strip reasoning/tool bloat
    Converts bloated metadata into compact tokens
    """
    if not content:
        return ""

    # Handle OpenAI-style content lists (multimodal)
    if isinstance(content, list):
        texts = []
        for part in content:
            if isinstance(part, dict):
                if part.get('type') == 'text':
                    texts.append(part.get('text', ''))
                elif part.get('type') == 'image_url' or part.get('type') == 'image':
                    texts.append('[image]')
                else:
                    texts.append(str(part))
            else:
                texts.append(str(part))
        content = '\n'.join(texts)

    if not isinstance(content, str):
        content = str(content)

    # Strip <reasoning> blocks
    content = re.sub(r'<reasoning>.*?</reasoning>', '[reasoning distilled]', content, flags=re.DOTALL)
    content = re.sub(r'reasoning_content:\s*.*?\n', '', content)
    
    # Condense tool results
    content = re.sub(r'tool_results:\s*\[.*?\]', '[tools executed]', content, flags=re.DOTALL)
    content = re.sub(r'Tool.*?executed.*?\n', '[tool executed]\n', content, flags=re.IGNORECASE)
    
    # Remove excessive whitespace
    content = re.sub(r'\n{3,}', '\n\n', content)
    
    return content.strip()


def rectify_context(messages: List[Dict[str, Any]], current_query: str = "", frozen_count: int = 0) -> tuple[List[Dict[str, Any]], RectificationStats]:
    """
    Perform alchemical rectification on message context.
    
    Strategies:
    - 'vram': Keep only recent messages (minimize VRAM usage)
    - 'ssd': Archive old messages to SSD before truncating
    - 'hybrid': Archive + keep recent messages in VRAM
    - 'stream': Archive + intelligently stream relevant context from SSD
    
    Operations:
    1. Calcination (Truncation): Keep system + last N messages
    2. Sublimation (Metadata Stripping): Remove reasoning/tool bloat
    3. Coagulation (Final Formatting): Ensure clean ChatML format
    4. Streaming (Optional): Inject relevant archived context
    """
    if not messages:
        return messages, RectificationStats(0, 0, 0, False, 0.0)
    
    original_messages = messages.copy()
    def _content_str(m):
        c = m.get('content', '')
        if isinstance(c, list):
            texts = []
            for part in c:
                if isinstance(part, dict) and part.get('type') == 'text':
                    texts.append(part.get('text', ''))
            c = ' '.join(texts)
        elif not isinstance(c, str):
            c = str(c)
        r = m.get('reasoning_content', '')
        if isinstance(r, list):
            texts = []
            for part in r:
                if isinstance(part, dict) and part.get('type') == 'text':
                    texts.append(part.get('text', ''))
            r = ' '.join(texts)
        elif not isinstance(r, str):
            r = str(r)
        return c + r
    original_tokens = sum(
        estimate_tokens(_content_str(m))
        for m in messages
    )
    
    # Separate frozen prefix from active messages
    active_messages = messages
    frozen_prefix = []
    if frozen_count > 0 and frozen_count <= len(messages):
        frozen_prefix = messages[:frozen_count]
        active_messages = messages[frozen_count:]
    
    # Context Offloading: Archive old messages to SSD (active messages only)
    if CONTEXT_STRATEGY in ['ssd', 'hybrid', 'stream'] and len(active_messages) > MAX_MESSAGES_TO_KEEP:
        messages_to_archive = active_messages[:-MAX_MESSAGES_TO_KEEP]
        archive_path = archive_context_to_ssd(messages_to_archive)
        if archive_path:
            logger.info(f"Archived {len(messages_to_archive)} messages to SSD")
    
    # Context Streaming: Inject relevant archived context
    streamed_messages = []
    if CONTEXT_STRATEGY == 'stream' and current_query:
        streamed_messages = stream_relevant_context(current_query)
        if streamed_messages:
            logger.info(f"Streaming {len(streamed_messages)} relevant context messages from SSD")
    
    # 1. Calcination: Truncate middle messages (active messages only)
    messages_dropped = 0
    if len(active_messages) > MAX_MESSAGES_TO_KEEP:
        # Always keep system prompt (first message if role=system)
        system_msg = None
        if active_messages[0].get('role') == 'system':
            system_msg = active_messages[0]
            active_messages = active_messages[1:]
        
        # Keep only the last N messages
        messages_dropped = len(active_messages) - MAX_MESSAGES_TO_KEEP
        active_messages = active_messages[-MAX_MESSAGES_TO_KEEP:]
        
        # Restore system message
        if system_msg:
            active_messages = [system_msg] + active_messages
    
    # Inject streamed context as system message
    if streamed_messages:
        streamed_text = "\n\n[Relevant Context from Archive]\n"
        for msg in streamed_messages[-5:]:  # Last 5 streamed messages
            streamed_text += f"{msg['role']}: {msg['content'][:200]}...\n"
        streamed_text += "[End Context]\n\n"
        
        # Add as system message or append to existing system message
        if active_messages and active_messages[0].get('role') == 'system':
            sc = active_messages[0]['content']
            if isinstance(sc, str):
                active_messages[0]['content'] = sc + streamed_text
            elif isinstance(sc, list):
                for part in sc:
                    if isinstance(part, dict) and part.get('type') == 'text':
                        part['text'] = part['text'] + streamed_text
                        break
                else:
                    sc.append({"type": "text", "text": streamed_text})
        else:
            active_messages.insert(0, {'role': 'system', 'content': streamed_text})
    
    # 2. Sublimation: Strip metadata from each message (active only)
    metadata_stripped = False
    for msg in active_messages:
        if 'content' in msg:
            original_len = len(msg['content'])
            msg['content'] = sublimate_content(msg['content'])
            if len(msg['content']) < original_len:
                metadata_stripped = True
        
        # Remove reasoning_content entirely (not needed for inference)
        if 'reasoning_content' in msg:
            del msg['reasoning_content']
            metadata_stripped = True
        
        # Clean up tool_calls if present (keep minimal info)
        if 'tool_calls' in msg:
            msg['tool_calls'] = [{'id': tc.get('id', '')} for tc in msg['tool_calls']]
            metadata_stripped = True
    
    # Reassemble: frozen prefix + rectified active messages
    messages = frozen_prefix + active_messages
    
    # Calculate final token count
    rectified_tokens = sum(
        estimate_tokens(_content_str(m))
        for m in messages
    )
    
    reduction_percent = ((original_tokens - rectified_tokens) / original_tokens * 100) if original_tokens > 0 else 0.0
    
    stats = RectificationStats(
        original_tokens=original_tokens,
        rectified_tokens=rectified_tokens,
        messages_dropped=messages_dropped,
        metadata_stripped=metadata_stripped,
        reduction_percent=reduction_percent
    )
    
    return messages, stats


def backend_status() -> Dict[str, Any]:
    """Check if llama-server is running and healthy"""
    try:
        resp = requests.get(LLAMA_STATUS_URL, timeout=5)
        if resp.status_code == 200:
            return {"status": "ok", "backend": resp.json()}
        return {"status": "error", "message": f"Backend returned {resp.status_code}"}
    except requests.exceptions.ConnectionError:
        return {"status": "error", "message": "llama-server not running"}
    except Exception as e:
        return {"status": "error", "message": str(e)}


@app.route('/v1/chat/completions', methods=['POST'])
def proxy_chat_completions():
    """
    OpenAI-compatible endpoint that rectifies context before forwarding to llama-server.
    Supports streaming: when the client sends stream=true, SSE chunks are proxied through.
    """
    try:
        data = request.json
        if not data:
            return jsonify({"error": "No JSON body"}), 400

        # === GUARDRAIL 4: Thermal Sentinel ===
        temp = get_gpu_temp()
        logger.info(f"GPU Temperature: {temp}°C")
        if temp >= MAX_TEMP:
            logger.error(f"THERMAL HALT: GPU at {temp}°C (limit: {MAX_TEMP}°C)")
            return jsonify({
                "error": f"Alchemical Overheat: {temp}°C. Cooling required.",
                "thermal_limit": MAX_TEMP
            }), 503

        messages = data.get('messages', [])
        logger.info(f"Received request with {len(messages)} messages")

        # Extract current query for context streaming
        current_query = ""
        if messages and messages[-1].get('role') == 'user':
            raw = messages[-1].get('content', '')
            if isinstance(raw, list):
                texts = []
                for part in raw:
                    if isinstance(part, dict) and part.get('type') == 'text':
                        texts.append(part.get('text', ''))
                current_query = ' '.join(texts)[:500]
            elif isinstance(raw, str):
                current_query = raw[:500]

        # ── Emulated Memory Intercept ──
        memory_candidates = []
        if MEMORY_MODE:
            project_id = request.headers.get('X-Project-Id', 'default')
            session_id = request.headers.get('X-Session-Id', 'default')
            emem.db.get_or_create_session(project_id, session_id)
            memory_candidates = emem.retrieve(project_id, current_query)
            recent = emem.db.get_recent_episodes(project_id, session_id, limit=2)
            memory_lines = emem.compact_context(
                candidates=memory_candidates, project_id=project_id,
                session_id=session_id, query=current_query, recent_episodes=recent,
            )
            memory_text = "\n".join(memory_lines)
            if memory_text.strip():
                if messages and messages[0].get('role') == 'system':
                    content = messages[0]['content']
                    if isinstance(content, str):
                        messages[0]['content'] = content + "\n\n" + memory_text
                    elif isinstance(content, list):
                        for part in content:
                            if isinstance(part, dict) and part.get('type') == 'text':
                                part['text'] = part['text'] + "\n\n" + memory_text
                                break
                        else:
                            content.append({"type": "text", "text": memory_text})
                else:
                    messages.insert(0, {'role': 'system', 'content': memory_text})
                logger.info(
                    f"MEMORY: Injected {emem.estimate_tokens(memory_text)} tokens "
                    f"from memory DB ({project_id}/{session_id})"
                )
            if emem.consolidate._consolidation_thread:
                emem.consolidate._consolidation_thread.mark_active()

        # ── Frozen Prompt Caching ──
        frozen_count = 0
        if FROZEN_PROMPT:
            for i, m in enumerate(messages):
                if m.get('role') == 'system' or m.get('role') == 'tool':
                    frozen_count = i + 1
                else:
                    break
            if frozen_count > 0:
                import hashlib
                frozen_text = '|'.join(
                    str(m.get('content', '')) + str(m.get('role', ''))
                    for m in messages[:frozen_count]
                )
                hashlib.md5(frozen_text.encode()).digest()  # hash, not used yet
                logger.info(f"FROZEN: {frozen_count} messages preserved as stable prefix")

        # === GUARDRAIL 1 & 2: Context Rectification with Streaming ===
        rectified_messages, stats = rectify_context(messages, current_query, frozen_count)
        logger.info(
            f"RECTIFICATION: {stats.original_tokens} -> {stats.rectified_tokens} tokens "
            f"({stats.reduction_percent:.1f}% reduction, dropped {stats.messages_dropped} messages)"
        )
        if stats.metadata_stripped:
            logger.info("Metadata stripped from context")

        data['messages'] = rectified_messages
        data['max_tokens'] = min(data.get('max_tokens', 1024), 1024)
        is_stream = data.get('stream', False)

        bstatus = backend_status()
        if bstatus['status'] != 'ok':
            logger.warning(f"Backend issue: {bstatus['message']}")

        logger.info(f"Forwarding to llama-server at {LLAMA_API_URL}")
        backend_resp = requests.post(
            LLAMA_API_URL, json=data,
            timeout=120, stream=is_stream,
        )

        if is_stream:
            return _proxy_stream(backend_resp, request, memory_candidates, current_query)
        else:
            return _proxy_buffered(backend_resp, request, memory_candidates, current_query)

    except requests.exceptions.Timeout:
        logger.error("Backend request timed out")
        return jsonify({"error": "Inference timeout"}), 504
    except Exception as e:
        logger.error(f"Proxy error: {e}")
        return jsonify({"error": str(e)}), 500


def _proxy_stream(backend_resp, req, memory_candidates, current_query):
    """Proxy SSE stream from llama-server back to client, then store the turn."""

    def generate():
        collected = ""
        try:
            for chunk in backend_resp.iter_content(chunk_size=None):
                if chunk:
                    decoded = chunk.decode('utf-8', errors='replace')
                    collected += decoded
                    # Log progress from llama-server SSE events
                    for line in decoded.split('\n'):
                        if '"progress"' in line or '"tokens_per_second"' in line:
                            logger.info(f"llama-server: {line.strip()}")
                    yield decoded
        finally:
            # After the stream ends, store the conversation turn
            if MEMORY_MODE:
                _store_turn(backend_resp.status_code, collected, req, memory_candidates, current_query)

    return Response(stream_with_context(generate()),
                    mimetype='text/event-stream',
                    headers={'Cache-Control': 'no-cache', 'X-Accel-Buffering': 'no'})


def _proxy_buffered(backend_resp, req, memory_candidates, current_query):
    """Buffer full response from llama-server, return JSON, then store turn."""
    response_data = backend_resp.json()
    if MEMORY_MODE and backend_resp.status_code == 200:
        _store_turn(200, json.dumps(response_data), req, memory_candidates, current_query)
    return jsonify(response_data), backend_resp.status_code


def _store_turn(status_code, raw_response, req, memory_candidates, current_query):
    """Store the conversation turn in the memory DB (with retries on lock)."""
    if status_code != 200 or not MEMORY_MODE:
        return
    project_id = req.headers.get('X-Project-Id', 'default')
    session_id = req.headers.get('X-Session-Id', 'default')
    try:
        response_data = json.loads(raw_response)
    except Exception:
        response_data = {}
    for _attempt in range(3):
        try:
            emem.db.store_episode(
                project_id, session_id, 'user',
                current_query if current_query else '(empty)',
                token_count=emem.estimate_tokens(current_query)
            )
            assistant_text = ''
            if 'choices' in response_data and response_data['choices']:
                choice = response_data['choices'][0]
                if 'message' in choice and 'content' in choice['message']:
                    assistant_text = choice['message']['content']
                elif 'text' in choice:
                    assistant_text = choice['text']
            if assistant_text:
                emem.db.store_episode(
                    project_id, session_id, 'assistant',
                    assistant_text,
                    token_count=emem.estimate_tokens(assistant_text)
                )
            if memory_candidates:
                emem.update_weights(project_id, assistant_text, memory_candidates)
            break
        except Exception as e:
            if _attempt < 2:
                logger.warning(f"MEMORY: retry store ({_attempt+1}/3): {e}")
                time.sleep(1)
            else:
                logger.warning(f"MEMORY: Failed to store turn after 3 attempts: {e}")


@app.route('/v1/models', methods=['GET'])
def proxy_models():
    """Proxy model listing endpoint"""
    try:
        response = requests.get("http://localhost:5001/v1/models", timeout=10)
        return jsonify(response.json()), response.status_code
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route('/health', methods=['GET'])
def health():
    """VITRIOL shim health check"""
    bstatus = backend_status()
    return jsonify({
        "status": "ok",
        "shim": "running",
        "memory_mode": MEMORY_MODE,
        "semantic_mode": SEMANTIC_MODE,
        "backend": bstatus,
        "config": {
            "max_context_tokens": MAX_CONTEXT_TOKENS,
            "max_messages": MAX_MESSAGES_TO_KEEP,
            "llama_api_url": LLAMA_API_URL,
            "context_strategy": CONTEXT_STRATEGY,
            "memory_mode": "on" if MEMORY_MODE else "off",
            "semantic_mode": "on" if SEMANTIC_MODE else "off",
        }
    })


@app.route('/rectify', methods=['POST'])
def rectify_only():
    """
    Debug endpoint: rectify context without inference.
    Useful for testing what VITRIOL would do to a prompt.
    """
    try:
        data = request.json
        messages = data.get('messages', [])
        rectified, stats = rectify_context(messages)
        
        return jsonify({
            "original_messages": len(messages),
            "rectified_messages": len(rectified),
            "stats": {
                "original_tokens": stats.original_tokens,
                "rectified_tokens": stats.rectified_tokens,
                "messages_dropped": stats.messages_dropped,
                "metadata_stripped": stats.metadata_stripped,
                "reduction_percent": stats.reduction_percent
            },
            "rectified_messages": rectified
        })
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route('/context/archive', methods=['POST'])
def archive_context_endpoint():
    """
    Manually archive context to SSD.
    Useful for long conversations where you want to preserve history.
    """
    try:
        data = request.json
        messages = data.get('messages', [])
        archive_path = data.get('path', '/tmp/vitriol_context_archive.json')
        
        result_path = archive_context_to_ssd(messages, archive_path)
        if result_path:
            return jsonify({
                "status": "ok",
                "archive_path": result_path,
                "messages_archived": len(messages)
            })
        else:
            return jsonify({"error": "Archival failed"}), 500
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route('/context/retrieve', methods=['GET'])
def retrieve_context_endpoint():
    """
    Retrieve archived context from SSD.
    Returns messages list for potential injection into conversation.
    """
    try:
        archive_path = request.args.get('path', '/tmp/vitriol_context_archive.json')
        messages = retrieve_context_from_ssd(archive_path)
        
        return jsonify({
            "status": "ok",
            "messages": messages,
            "message_count": len(messages)
        })
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route('/memory/stats', methods=['GET'])
def memory_stats():
    """Emulated memory statistics endpoint."""
    if not MEMORY_MODE:
        return jsonify({"error": "Memory mode not enabled"}), 400

    try:
        project_id = request.headers.get('X-Project-Id', 'default')
        session_id = request.headers.get('X-Session-Id', 'default')

        conn = emem.db._get_conn(project_id)
        episodes = conn.execute("SELECT COUNT(*) as c FROM episodes").fetchone()
        nodes = conn.execute("SELECT COUNT(*) as c FROM knowledge_nodes").fetchone()
        edges = conn.execute("SELECT COUNT(*) as c FROM edges").fetchone()
        session = conn.execute(
            "SELECT * FROM sessions WHERE session_id = ?", (session_id,)
        ).fetchone()

        return jsonify({
            "project_id": project_id,
            "session_id": session_id,
            "episode_count": episodes['c'] if episodes else 0,
            "node_count": nodes['c'] if nodes else 0,
            "edge_count": edges['c'] if edges else 0,
            "session_turns": session['turn_count'] if session else 0,
        })
    except Exception as e:
        return jsonify({"error": str(e)}), 500


@app.route('/memory/clear', methods=['POST'])
def memory_clear():
    """Clear memory database for a project."""
    if not MEMORY_MODE:
        return jsonify({"error": "Memory mode not enabled"}), 400

    try:
        project_id = request.headers.get('X-Project-Id', 'default')
        confirm = request.json.get('confirm', False) if request.json else False
        if not confirm:
            return jsonify({"error": "Must set confirm=true to clear memory"}), 400

        import shutil
        memory_dir = os.environ.get('VITRIOL_MEMORY_DIR',
                                     os.path.expanduser('~/.vitriol'))
        project_dir = os.path.join(memory_dir, project_id)
        if os.path.isdir(project_dir):
            shutil.rmtree(project_dir)
            logger.info(f"MEMORY: Cleared database for project '{project_id}'")
            return jsonify({"status": "ok", "message": f"Memory cleared for '{project_id}'"})
        return jsonify({"status": "ok", "message": "No memory to clear"})
    except Exception as e:
        return jsonify({"error": str(e)}), 500


def main():
    """Run the VITRIOL shim"""
    logger.info(f"VITRIOL Context Rectifier starting on port {SHIM_PORT}")
    logger.info(f"Forwarding to llama-server at {LLAMA_API_URL}")
    logger.info(f"Max context: {MAX_CONTEXT_TOKENS} tokens, Max messages: {MAX_MESSAGES_TO_KEEP}")
    
    app.run(host='0.0.0.0', port=SHIM_PORT, debug=False, threaded=True)


if __name__ == '__main__':
    main()
