#!/usr/bin/env python3
"""Copula service — VITRIOL memory HTTP API for the OpenCode Copula plugin.

Part of the Copula subsystem (the VITRIOL-to-OpenCode bond). Endpoints
(localhost only):
  POST /memory/store     store an episode (role: user|assistant|tool)
  POST /memory/node      upsert a knowledge node (repo-map/file entries, keyed by label)
  POST /memory/search    multi-hop retrieval, returns formatted snippets
  GET  /memory/stats     per-project stats (episodes, nodes, sessions)
  GET  /health           liveness

Reuses libvitriol/memory (db, retrieval, compact). Semantic embeddings are wired in
P2 (GPU GGUF via llama-server /embedding); until then keyword+recency scoring runs
(no VITRIOL_SEMANTIC_MODE needed).
"""
import argparse
import json
import os
import sys

# Allow running as a standalone script or from the repo tree.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from flask import Flask, jsonify, request

from memory import compact, db, retrieval

app = Flask(__name__)

# Bound to localhost only; the plugin talks to it over loopback.
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 8090


def _project_id(payload):
    """Resolve project_id from payload; reject requests without one."""
    pid = payload.get("project_id") or payload.get("project")
    if not pid:
        return None
    # Sanitize: project id becomes a directory name under ~/.vitriol
    pid = pid.replace("/", "_").replace("\\", "_").replace(":", "_")
    return pid[:120]


@app.route("/health", methods=["GET"])
def health():
    """Liveness probe for the plugin and operators."""
    return jsonify({"status": "ok", "service": "copula"})


@app.route("/memory/store", methods=["POST"])
def memory_store():
    """Store a conversation turn or tool result as an episode."""
    payload = request.get_json(force=True, silent=True) or {}
    pid = _project_id(payload)
    if not pid:
        return jsonify({"error": "project_id required"}), 400
    role = payload.get("role", "assistant")
    content = payload.get("content", "")
    session_id = payload.get("session_id", "default")
    if not content:
        return jsonify({"error": "content required"}), 400
    token_count = payload.get("token_count", 0)
    db.get_or_create_session(pid, session_id)
    episode_id = db.store_episode(pid, session_id, role, content,
                                  meta={'token_count': token_count})
    return jsonify({"ok": True, "episode_id": episode_id, "project_id": pid})


@app.route("/memory/node", methods=["POST"])
def memory_node():
    """Upsert a knowledge node keyed by label (e.g. file path -> summary)."""
    payload = request.get_json(force=True, silent=True) or {}
    pid = _project_id(payload)
    if not pid:
        return jsonify({"error": "project_id required"}), 400
    label = payload.get("label", "")
    summary = payload.get("summary", "")
    if not label or not summary:
        return jsonify({"error": "label and summary required"}), 400
    node_id = db.store_node(pid, label, summary,
                            meta={'strength': payload.get('strength', 1.0),
                                  'source_min': payload.get('source_min'),
                                  'source_max': payload.get('source_max')})
    return jsonify({"ok": True, "node_id": node_id, "project_id": pid})


@app.route("/memory/search", methods=["POST"])
def memory_search():
    """Multi-hop retrieval; returns formatted snippets for context injection."""
    payload = request.get_json(force=True, silent=True) or {}
    pid = _project_id(payload)
    if not pid:
        return jsonify({"error": "project_id required"}), 400
    query = payload.get("query", "")
    if not query:
        return jsonify({"error": "query required"}), 400
    top_k = int(payload.get("top_k", 5))
    cascade_depth = int(payload.get("cascade_depth", 1))

    candidates = retrieval.retrieve(pid, query, top_k=top_k,
                                    cascade_depth=cascade_depth)
    results = []
    for c in candidates:
        ctype = c.get("_type", "episode")
        if ctype == "node":
            body = compact.format_node(c)
        else:
            body = compact.format_episode(c)
        results.append({
            "type": ctype,
            "content": body,
            "score": round(c.get("_score", 0.0), 4),
            "source": c.get("_source", ""),
        })
    return jsonify({"ok": True, "query": query, "results": results,
                    "count": len(results), "project_id": pid})


@app.route("/memory/stats", methods=["GET"])
def memory_stats():
    """Return per-project episode/node/session counts."""
    pid = _project_id(request.args)
    if not pid:
        return jsonify({"error": "project_id required"}), 400
    conn = db._get_conn(pid)
    episodes = conn.execute("SELECT COUNT(*) FROM episodes").fetchone()[0]
    nodes = conn.execute("SELECT COUNT(*) FROM knowledge_nodes").fetchone()[0]
    sessions = conn.execute("SELECT COUNT(*) FROM sessions").fetchone()[0]
    return jsonify({"project_id": pid, "episodes": episodes,
                    "nodes": nodes, "sessions": sessions})


def main():
    """Parse args and run the service on localhost."""
    ap = argparse.ArgumentParser(description="VITRIOL memory service")
    ap.add_argument("--host", default=DEFAULT_HOST)
    ap.add_argument("--port", type=int, default=DEFAULT_PORT)
    ap.add_argument("--debug", action="store_true")
    args = ap.parse_args()
    app.run(host=args.host, port=args.port, debug=args.debug)


if __name__ == "__main__":
    main()
