#!/usr/bin/env python3
"""repo-map persistent dispatcher — newline-delimited JSON over stdio.

Why this exists (2026-08-29, Trismegistus step 5): repo-map's server.py is a
FastMCP stdio server whose module import costs ~4s. One-shot invocations from
the scaffold would pay that per tool call, so this shim keeps ONE warm process
and answers simple JSONL requests instead of the full MCP handshake. It IMPORTS
repo-map (via sys.path) and never patches it — upstream stays upstream.

Protocol (one JSON object per line, both directions):
  in : {"id": <int>, "cmd": "index|where_is|grep_code|outline|get_symbol|who_references|what_it_uses|refresh|ping", "args": {...}}
  out: {"id": <int>, "ok": true,  "text": "<markdown result>"}
       {"id": <int>, "ok": false, "error": "<message>"}

Launch: python shim.py <repo-map-dir> [initial-repo-to-index]
"""
import importlib.util
import inspect
import json
import sys
import traceback


def load_server(repomap_dir):
    """Import repo-map's server.py from its own directory (sibling imports need cwd on path)."""
    sys.path.insert(0, repomap_dir)
    spec = importlib.util.spec_from_file_location(
        "repo_map_server", repomap_dir + "/server.py"
    )
    mod = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(mod)
    except SystemExit:
        pass  # FastMCP may exit on import-time arg parsing; module state is still usable
    return mod


def unwrap(fn):
    """FastMCP decorators expose the raw function as .fn."""
    return getattr(fn, "fn", fn)


def run_anyway(x):
    """Await an awaitable from sync code via a fresh event loop."""
    if not inspect.isawaitable(x):
        return x
    import anyio

    async def _go():
        return await x

    return anyio.run(_go)


TOOL_CMDS = (
    "index", "where_is", "grep_code", "outline",
    "get_symbol", "who_references", "what_it_uses", "refresh",
)


def handle(mod, req):
    """Execute one request dict and return the reply dict. Never raises."""
    req_id = req.get("id")
    cmd = req.get("cmd", "ping")
    args = req.get("args", {}) or {}
    try:
        if cmd == "ping":
            return {"id": req_id, "ok": True, "text": "pong"}
        if cmd not in TOOL_CMDS:
            raise KeyError("unknown cmd: " + str(cmd))
        out = run_anyway(unwrap(getattr(mod, cmd))(**args))
        return {"id": req_id, "ok": True, "text": str(out)}
    except Exception:  # noqa: BLE001 — the loop must survive any tool failure
        return {"id": req_id, "ok": False, "error": traceback.format_exc(limit=4)}


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"id": 0, "ok": False, "error": "usage: shim.py <repomap-dir> [repo]"}))
        return 2
    repomap_dir = sys.argv[1]
    mod = load_server(repomap_dir)
    if len(sys.argv) > 2:
        unwrap(mod.index)(sys.argv[2])
    sys.stderr.write("READY\n")
    sys.stderr.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            req = {"id": None}
            print(json.dumps({"id": None, "ok": False, "error": "invalid JSON on stdin"}), flush=True)
            continue
        print(json.dumps(handle(mod, req)), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
