#!/usr/bin/env python3
"""Hermetis server — the memory system's HTTP API for the OpenCode plugin.

Hermetis is VITRIOL's memory system (the persistent RAG brain). This service is its
network facade. The OpenCode plugin that connects to it is the **Copula Hermetis** (the
Copula bond into Hermetis). Endpoints (localhost only):
  POST /hermetis/store     store an episode (role: user|assistant|tool)
  POST /hermetis/node      upsert a knowledge node (repo-map/file entries, keyed by label)
  POST /hermetis/search    multi-hop retrieval, returns formatted snippets
  GET  /hermetis/stats     per-project stats (episodes, nodes, sessions)
  GET  /health           liveness

Reuses libvitriol/hermetis (db, retrieval, compact). Semantic embeddings are wired in
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

from hermetis import compact, db, retrieval

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
    return jsonify({"status": "ok", "service": "hermetis"})


@app.route("/hermetis/store", methods=["POST"])
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
    # 2026-08-06: warm the embedding cache on store when the GPU embedder is up,
    # so semantic retrieval is fast (lazy compute stays the fallback).
    try:
        from hermetis import embed
        if embed.is_available():
            embed.encode(content)
    except Exception:
        pass
    return jsonify({"ok": True, "episode_id": episode_id, "project_id": pid})


@app.route("/hermetis/embed", methods=["POST"])
def hermetis_embed():
    """Compute (and cache) an embedding via the GPU GGUF provider."""
    payload = request.get_json(force=True, silent=True) or {}
    text = payload.get("text", "")
    if not text:
        return jsonify({"error": "text required"}), 400
    from hermetis import embed
    if not embed.is_available():
        return jsonify({"ok": False, "error": "embed server unavailable"}), 503
    vec = embed.encode(text)
    if vec is None:
        return jsonify({"ok": False, "error": "embed failed"}), 502
    return jsonify({"ok": True, "dims": len(vec),
                    "preview": [round(x, 4) for x in vec[:8]],
                    "norm": round(sum(x * x for x in vec) ** 0.5, 4)})


@app.route("/hermetis/node", methods=["POST"])
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


@app.route("/hermetis/search", methods=["POST"])
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
    include_history = bool(payload.get("include_history", False))

    candidates = retrieval.retrieve(pid, query, top_k=top_k,
                                    cascade_depth=cascade_depth,
                                    include_history=include_history)
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


@app.route("/hermetis/repo_map", methods=["POST"])
def hermetis_repo_map():
    """Build (and optionally store) the Aider-style repo map for a project root."""
    payload = request.get_json(force=True, silent=True) or {}
    pid = _project_id(payload)
    if not pid:
        return jsonify({"error": "project_id required"}), 400
    root = payload.get("root", "")
    if not root or not os.path.isdir(root):
        return jsonify({"error": "root must be a directory"}), 400
    budget = int(payload.get("budget_tokens", 1000))
    do_store = bool(payload.get("store", True))
    max_files = payload.get("max_files")
    single_file = payload.get("file")
    from hermetis import repomap
    if single_file:
        # targeted refresh of one file (file-edit trigger, P3.4)
        stored = repomap.store_file_nodes(pid, root, [single_file])
        map_text = repomap.build_repo_map(root, budget, max_files)
    elif do_store:
        map_text, stored = repomap.store_repo_map(pid, root, budget, max_files)
    else:
        map_text = repomap.build_repo_map(root, budget, max_files)
        stored = 0
    return jsonify({"ok": True, "project_id": pid, "nodes_stored": stored,
                    "map_tokens": repomap.estimate_tokens(map_text), "map": map_text})


@app.route("/hermetis/context", methods=["POST"])
def hermetis_context():
    """Build a budget-capped context block for auto-injection (rolling window, C)."""
    payload = request.get_json(force=True, silent=True) or {}
    pid = _project_id(payload)
    if not pid:
        return jsonify({"error": "project_id required"}), 400
    recent_text = payload.get("recent_text", "")
    if not recent_text:
        return jsonify({"error": "recent_text required"}), 400
    budget = int(payload.get("budget_tokens", 3000))
    top_k = int(payload.get("top_k", 5))
    block = retrieval.context_block(pid, recent_text, budget, top_k)
    return jsonify({"ok": True, "project_id": pid,
                    "tokens": retrieval.estimate_tokens(block),
                    "context": block})


@app.route("/hermetis/stats", methods=["GET"])
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
