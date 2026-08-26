#!/usr/bin/env python3
"""vitriol_rag — semantic recall over the Pymander mirror (:8282).

Endpoints:
  GET  /health            {ok, nodes, model_loaded}
  GET  /search?q=&top_k=  cross-domain cosine ranking: [{domain,label,score,summary}]
  POST /sync              re-run markdown→mongo sync, then refresh vector cache
  GET  /stats             corpus + cache stats

Design notes:
- Embeddings are LOCAL (fastembed bge-small int8) — no cloud dependency for
  internal knowledge lookups.
- Vector cache lives in memory; refreshed every CACHE_TTL or on /sync. At
  this corpus size (hundreds of nodes) client-side cosine beats any vector
  index infrastructure.
- Every failure mode degrades to empty results — callers (copula.ts) must
  treat RAG as an augmentation, never a dependency.
"""
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pymander_sync as sync  # noqa: E402

MONGO_URI = sync.MONGO_URI
PORT = int(os.environ.get("VITRIOL_RAG_PORT", "8282"))
CACHE_TTL = float(os.environ.get("VITRIOL_RAG_CACHE_TTL", "300"))

_state = {"docs": [], "loaded_at": 0.0, "model": None}


def log(msg):
    print(f"[rag {time.strftime('%H:%M:%S')}] {msg}", flush=True)


def get_model():
    if _state["model"] is None:
        from fastembed import TextEmbedding
        t0 = time.time()
        _state["model"] = TextEmbedding(model_name=sync.MODEL_NAME)
        log(f"embedder loaded ({sync.MODEL_NAME}) in {time.time()-t0:.1f}s")
    return _state["model"]


def refresh_cache(force=False):
    """Pull id+embedding+meta docs from Mongo into memory. On Mongo failure
    with a warm cache, serve stale and retry in 30 s — augmentation must
    never turn a transient store outage into caller-visible errors."""
    now = time.time()
    if not force and _state["docs"] and now - _state["loaded_at"] < CACHE_TTL:
        return
    try:
        from pymongo import MongoClient
        col = MongoClient(MONGO_URI, serverSelectionTimeoutMS=3000)[
            "vitriol_pymander"]["nodes"]
        _state["docs"] = list(col.find(
            {}, {"domain": 1, "label": 1, "summary": 1, "embedding": 1}))
        _state["loaded_at"] = now
        log(f"cache refreshed: {len(_state['docs'])} vectors")
    except Exception as e:
        if _state["docs"]:
            # serve stale; back off the TTL so we don't hammer a dead store
            _state["loaded_at"] = now - CACHE_TTL + 30
            log(f"mongo unreachable — serving stale ({len(_state['docs'])} "
                f"vectors): {e}")
        else:
            raise


def cosine(a, b):
    dot = sum(x * y for x, y in zip(a, b))
    na = sum(x * x for x, y in zip(a, a)) ** 0.5 or 1e-9
    nb = sum(y * y for y in b) ** 0.5 or 1e-9
    return dot / (na * nb)


def search(q, top_k):
    if not q.strip():
        return []
    refresh_cache()
    if not _state["docs"]:
        return []
    qv = list(get_model().embed([q], batch_size=1))[0]
    scored = []
    for d in _state["docs"]:
        emb = d.get("embedding")
        if not emb:
            continue
        scored.append((cosine(qv, emb), d))
    scored.sort(key=lambda t: -t[0])
    return [
        {
            "domain": d.get("domain", "?"),
            "label": d.get("label", "?"),
            "summary": d.get("summary", ""),
            "score": round(float(s), 4),
        }
        for s, d in scored[: max(1, top_k)]
    ]


class Handler(BaseHTTPRequestHandler):
    def _send(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urlparse(self.path)
        try:
            if u.path == "/health":
                self._send(200, {
                    "ok": True,
                    "nodes": len(_state["docs"]),
                    "model_loaded": _state["model"] is not None,
                })
            elif u.path == "/search":
                qs = parse_qs(u.query)
                q = (qs.get("q") or [""])[0]
                top_k = int((qs.get("top_k") or ["3"])[0])
                self._send(200, {"results": search(q, top_k)})
            elif u.path == "/stats":
                self._send(200, {
                    "cached_vectors": len(_state["docs"]),
                    "cache_age_s": round(time.time() - _state["loaded_at"], 1),
                    "model": sync.MODEL_NAME,
                    "mongo": MONGO_URI,
                })
            else:
                self._send(404, {"error": "not found"})
        except Exception as e:
            log(f"GET {u.path} failed: {e}")
            self._send(502, {"error": str(e)})

    def do_POST(self):
        if urlparse(self.path).path == "/sync":
            try:
                rc = sync.main([])  # in-process; argv-free incremental sync
                refresh_cache(force=True)
                self._send(200, {"ok": rc == 0})
            except Exception as e:
                log(f"sync failed: {e}")
                self._send(502, {"error": str(e)})
        else:
            self._send(404, {"error": "not found"})

    def log_message(self, format, *args):  # noqa: A002 - stdlib signature
        pass


if __name__ == "__main__":
    # pre-warm corpus lazily; model loads on first real query
    try:
        refresh_cache()
    except Exception as e:
        log(f"initial cache load failed (will retry per-request): {e}")
    log(f"listening on 127.0.0.1:{PORT}")
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
