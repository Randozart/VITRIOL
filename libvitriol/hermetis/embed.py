"""Hermetis embedding provider — GPU GGUF via llama-server /v1/embeddings.

P2 (2026-08-06): replaces sentence-transformers with a small GGUF embedding model
served by llama-server on the GPU. Falls back gracefully if the embed server is
unreachable or semantic mode is off (the caller keeps the keyword path).
"""
import hashlib
import json
import os
import threading
import urllib.request

EMBED_URL = os.environ.get('VITRIOL_EMBED_URL', 'http://127.0.0.1:8081')
EMBED_MODEL = os.environ.get('VITRIOL_EMBED_MODEL', 'nomic-embed-text-v1.5')

_CACHE = {}
_CACHE_LOCK = threading.Lock()
_CACHE_MAX = 4096

# bge-small-en-v1.5's native context is 512 tokens (llama-server clamps slots to it,
# and embeddings beyond the trained window are degraded anyway). Cap inputs at ~1800
# chars (~450 tokens) so large tool results embed instead of 500ing.
EMBED_MAX_CHARS = 1800


def _truncate(text):
    """Truncate text to the embedding model's context window."""
    if len(text) <= EMBED_MAX_CHARS:
        return text
    return text[:EMBED_MAX_CHARS]


def is_available():
    """True if the GGUF embed server responds and semantic mode is enabled."""
    if os.environ.get('VITRIOL_SEMANTIC_MODE', 'off').lower() != 'on':
        return False
    try:
        with urllib.request.urlopen(EMBED_URL + '/health', timeout=1.5) as resp:
            return resp.status == 200
    except Exception:
        return False


def encode(text):
    """Embed one text via the GGUF server; returns a list of floats or None."""
    text = _truncate(text)
    key = hashlib.sha256(text.encode('utf-8', errors='replace')).hexdigest()
    with _CACHE_LOCK:
        cached = _CACHE.get(key)
    if cached is not None:
        return cached
    try:
        body = json.dumps({'input': text, 'model': EMBED_MODEL}).encode('utf-8')
        req = urllib.request.Request(
            EMBED_URL + '/v1/embeddings', data=body,
            headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read())
        emb = data['data'][0]['embedding']
        # 2026-08-06: zero-guard. The fork's BERT-family embedding forward pass
        # returns all-zero vectors for certain token sequences (backend- and
        # pooling-independent). A zero vector poisons cosine similarity — treat it
        # as a failed embed so the caller falls back to keyword scoring.
        if not isinstance(emb, list) or len(emb) < 4:
            return None
        if sum(x * x for x in emb) < 1e-8:
            return None
        with _CACHE_LOCK:
            if len(_CACHE) >= _CACHE_MAX:
                _CACHE.clear()
            _CACHE[key] = emb
        return emb
    except Exception:
        return None
