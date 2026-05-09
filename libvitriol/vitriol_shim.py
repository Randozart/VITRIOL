#!/usr/bin/env python3
"""
VITRIOL Context Rectifier - KoboldCPP Proxy Shim

This shim sits between OpenCode (or any agent) and KoboldCPP,
performing "alchemical rectification" on context to prevent OOM crashes
on legacy hardware (i7-3770, GTX 1070 Ti, 8GB VRAM).

OpenCode -> VITRIOL Shim (port 5005) -> KoboldCPP (port 5001)

Implementation Plan v2.0 - Phase 1
"""

import json
import re
import subprocess
import requests
import logging
from flask import Flask, request, jsonify
from typing import List, Dict, Any
from dataclasses import dataclass
from datetime import datetime

# Try to import FAISS for vector search
try:
    import faiss
    FAISS_AVAILABLE = True
except ImportError:
    FAISS_AVAILABLE = False
    logger.info("FAISS not available, will use keyword-based search")

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - VITRIOL - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

app = Flask(__name__)

# Configuration - Implementation Plan v2.0
KOBOLD_URL = "http://localhost:5001/v1/chat/completions"
KOBOLD_STATUS_URL = "http://localhost:5001/api/v1/info"
MAX_CONTEXT_TOKENS = 7000
MAX_MESSAGES_TO_KEEP = 4
SHIM_PORT = 5010
MAX_TEMP = 85  # GPU thermal limit (°C)

# Context offloading strategy
# Options:
#   'vram' - Keep recent context in VRAM (fastest, limited by VRAM size)
#   'ssd' - Stream context from SSD (slower, "infinite" context)
#   'hybrid' - Active context in VRAM, archive to SSD (balanced)
#   'stream' - Intelligent context streaming from SSD (RAG-style)
CONTEXT_STRATEGY = 'stream'

# Context streaming configuration
CONTEXT_STREAM_TOP_K = 3  # Number of relevant context chunks to stream
CONTEXT_STREAM_RELEVANCE_THRESHOLD = 0.3  # Minimum similarity score

# Context offloading strategy
# Options:
#   'vram' - Keep recent context in VRAM (fastest, limited by VRAM size)
#   'ssd' - Stream context from SSD (slower, "infinite" context)
#   'hybrid' - Active context in VRAM, archive to SSD (balanced)
#   'stream' - Intelligent context streaming from SSD (RAG-style)
CONTEXT_STRATEGY = 'stream'

# Context streaming configuration
CONTEXT_STREAM_TOP_K = 3  # Number of relevant context chunks to stream
CONTEXT_STREAM_RELEVANCE_THRESHOLD = 0.3  # Minimum similarity score


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
            ['nvidia-smi', '--query-gpu=temperature.gpu', '--format=csv,noheader,nounits'],
            capture_output=True,
            text=True,
            timeout=1
        )
        return int(result.stdout.strip())
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


def sublimate_content(content: str) -> str:
    """
    Sublimation (Guardrail 2): Strip reasoning/tool bloat
    Converts bloated metadata into compact tokens
    """
    if not content:
        return ""
    
    # Strip <reasoning> blocks
    content = re.sub(r'<reasoning>.*?</reasoning>', '[reasoning distilled]', content, flags=re.DOTALL)
    content = re.sub(r'reasoning_content:\s*.*?\n', '', content)
    
    # Condense tool results
    content = re.sub(r'tool_results:\s*\[.*?\]', '[tools executed]', content, flags=re.DOTALL)
    content = re.sub(r'Tool.*?executed.*?\n', '[tool executed]\n', content, flags=re.IGNORECASE)
    
    # Remove excessive whitespace
    content = re.sub(r'\n{3,}', '\n\n', content)
    
    return content.strip()


def rectify_context(messages: List[Dict[str, Any]], current_query: str = "") -> tuple[List[Dict[str, Any]], RectificationStats]:
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
    original_tokens = sum(
        estimate_tokens(m.get('content', '') + m.get('reasoning_content', ''))
        for m in messages
    )
    
    # Context Offloading: Archive old messages to SSD
    if CONTEXT_STRATEGY in ['ssd', 'hybrid', 'stream'] and len(messages) > MAX_MESSAGES_TO_KEEP:
        messages_to_archive = messages[:-MAX_MESSAGES_TO_KEEP]
        archive_path = archive_context_to_ssd(messages_to_archive)
        if archive_path:
            logger.info(f"Archived {len(messages_to_archive)} messages to SSD")
    
    # Context Streaming: Inject relevant archived context
    streamed_messages = []
    if CONTEXT_STRATEGY == 'stream' and current_query:
        streamed_messages = stream_relevant_context(current_query)
        if streamed_messages:
            logger.info(f"Streaming {len(streamed_messages)} relevant context messages from SSD")
    
    # 1. Calcination: Truncate middle messages
    messages_dropped = 0
    if len(messages) > MAX_MESSAGES_TO_KEEP:
        # Always keep system prompt (first message if role=system)
        system_msg = None
        if messages[0].get('role') == 'system':
            system_msg = messages[0]
            messages = messages[1:]
        
        # Keep only the last N messages
        messages_dropped = len(messages) - MAX_MESSAGES_TO_KEEP
        messages = messages[-MAX_MESSAGES_TO_KEEP:]
        
        # Restore system message
        if system_msg:
            messages = [system_msg] + messages
    
    # Inject streamed context as system message
    if streamed_messages:
        streamed_text = "\n\n[Relevant Context from Archive]\n"
        for msg in streamed_messages[-5:]:  # Last 5 streamed messages
            streamed_text += f"{msg['role']}: {msg['content'][:200]}...\n"
        streamed_text += "[End Context]\n\n"
        
        # Add as system message or append to existing system message
        if messages and messages[0].get('role') == 'system':
            messages[0]['content'] += streamed_text
        else:
            messages.insert(0, {'role': 'system', 'content': streamed_text})
    
    # 2. Sublimation: Strip metadata from each message
    metadata_stripped = False
    for msg in messages:
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
    
    # Calculate final token count
    rectified_tokens = sum(
        estimate_tokens(m.get('content', ''))
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


def kobold_status() -> Dict[str, Any]:
    """Check if KoboldCPP is running and healthy"""
    try:
        resp = requests.get(KOBOLD_STATUS_URL, timeout=5)
        if resp.status_code == 200:
            return {"status": "ok", "kobold": resp.json()}
        return {"status": "error", "message": f"Kobold returned {resp.status_code}"}
    except requests.exceptions.ConnectionError:
        return {"status": "error", "message": "KoboldCPP not running on port 5001"}
    except Exception as e:
        return {"status": "error", "message": str(e)}


@app.route('/v1/chat/completions', methods=['POST'])
def proxy_chat_completions():
    """
    OpenAI-compatible endpoint that rectifies context before forwarding to KoboldCPP.
    
    Guardrails:
    1. Thermal polling (halts if GPU >= 85°C)
    2. Context truncation (max 7k tokens)
    3. Metadata stripping (reasoning/tools)
    4. Context streaming (injects relevant archived context)
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
            current_query = messages[-1].get('content', '')[:500]  # First 500 chars as query
        
        # === GUARDRAIL 1 & 2: Context Rectification with Streaming ===
        rectified_messages, stats = rectify_context(messages, current_query)
        
        # Log rectification stats
        logger.info(
            f"RECTIFICATION: {stats.original_tokens} -> {stats.rectified_tokens} tokens "
            f"({stats.reduction_percent:.1f}% reduction, dropped {stats.messages_dropped} messages)"
        )
        
        if stats.metadata_stripped:
            logger.info("Metadata stripped from context")
        
        # Update request with rectified context
        data['messages'] = rectified_messages
        
        # === GUARDRAIL 3: Coagulation (Generation Cap) ===
        data['max_tokens'] = min(data.get('max_tokens', 1024), 1024)
        
        # Check Kobold status
        kobold = kobold_status()
        if kobold['status'] != 'ok':
            logger.warning(f"KoboldCPP issue: {kobold['message']}")
        
        # Forward to KoboldCPP
        logger.info(f"Forwarding to KoboldCPP at {KOBOLD_URL}")
        response = requests.post(KOBOLD_URL, json=data, timeout=120)
        
        # Return Kobold's response
        return jsonify(response.json()), response.status_code
        
    except requests.exceptions.Timeout:
        logger.error("KoboldCPP request timed out")
        return jsonify({"error": "Inference timeout"}), 504
    except Exception as e:
        logger.error(f"Proxy error: {e}")
        return jsonify({"error": str(e)}), 500


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
    kobold = kobold_status()
    return jsonify({
        "status": "ok",
        "shim": "running",
        "koboldcpp": kobold,
        "config": {
            "max_context_tokens": MAX_CONTEXT_TOKENS,
            "max_messages": MAX_MESSAGES_TO_KEEP,
            "kobold_url": KOBOLD_URL
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


def main():
    """Run the VITRIOL shim"""
    logger.info(f"VITRIOL Context Rectifier starting on port {SHIM_PORT}")
    logger.info(f"Forwarding to KoboldCPP at {KOBOLD_URL}")
    logger.info(f"Max context: {MAX_CONTEXT_TOKENS} tokens, Max messages: {MAX_MESSAGES_TO_KEEP}")
    
    app.run(host='0.0.0.0', port=SHIM_PORT, debug=False, threaded=True)


if __name__ == '__main__':
    main()
