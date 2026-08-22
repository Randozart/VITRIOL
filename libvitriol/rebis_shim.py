#!/usr/bin/env python3
"""REBIS shim — transparent steering proxy in front of Mellum-direct agents.

Sits between an OpenAI-compatible agent harness (hermes) and the fast drafter
server. Forwards every request to Mellum, then watches responses for the
known under-initiation failure modes (no tool calls mid-task, premature final
answers, empty/short or repeated output). Flagged turns get a Qwen steering
verdict; incomplete work is continued by nudging Mellum once with explicit
orders, falling back to a Qwen-authored override.

The client sees exactly one ordinary model. Streaming requests are answered
with synthesized SSE from the buffered response.
"""

import argparse
import hashlib
import json
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# ── Steering verdict schema (Qwen-constrained) ───────────────────────

STEER_SCHEMA = {
    "type": "object",
    "properties": {
        "complete": {"type": "boolean"},
        "reasoning": {"type": "string"},
        "missing_actions": {"type": "array", "items": {"type": "string"}},
    },
    "required": ["complete", "missing_actions"],
}


# ── Pure heuristics (unit-tested) ────────────────────────────────────

def session_key(messages: list[dict]) -> str:
    """Conversation identity = hash of the first user message."""
    for m in messages:
        if m.get("role") == "user":
            content = m.get("content") or ""
            if isinstance(content, list):  # multimodal parts
                content = json.dumps(content)
            return hashlib.sha256(content.encode()).hexdigest()[:16]
    return "no-user"


def has_tool_calls(message: dict) -> bool:
    calls = message.get("tool_calls")
    return bool(calls)


def looks_final(message: dict) -> bool:
    """Assistant turn that reads like a terminal answer: prose, no tools."""
    if has_tool_calls(message):
        return False
    content = message.get("content") or ""
    return len(content.strip()) > 0


def flag_turn(message: dict, state: dict, min_final_len: int = 40) -> list[str]:
    """Return flag codes for an assistant turn given session state."""
    flags = []
    content = message.get("content") or ""
    if not has_tool_calls(message):
        if state.get("saw_tool_call"):
            flags.append("no-tool-after-tools")
        elif state.get("turn_count", 0) >= 1 and len(content.strip()) < min_final_len:
            flags.append("short-response")
    prev = state.get("last_content_hash")
    if prev and prev == hashlib.sha256(content.encode()).hexdigest():
        flags.append("repeated-content")
    return flags


def update_state(state: dict, message: dict) -> None:
    if has_tool_calls(message):
        state["saw_tool_call"] = True
    state["last_content_hash"] = hashlib.sha256(
        (message.get("content") or "").encode()).hexdigest()
    state["turn_count"] = state.get("turn_count", 0) + 1


NUDGE_TEMPLATE = (
    "Your previous reply was evaluated as INCOMPLETE for this task.\n"
    "Missing actions:\n{actions}\n\n"
    "Continue now. Use the available tools to perform the missing actions — "
    "do NOT describe them, call them. If truly nothing remains, say DONE."
)

OVERRIDE_SCHEMA = {
    "type": "object",
    "properties": {
        "explanation": {"type": "string"},
        "tool_calls": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "arguments_json": {"type": "string"},
                },
                "required": ["name", "arguments_json"],
            },
        },
    },
    "required": ["tool_calls"],
}


# ── Upstream HTTP ────────────────────────────────────────────────────

class Upstream:
    def __init__(self, mellum_url: str, qwen_url: str):
        self.mellum = mellum_url.rstrip("/")
        self.qwen = qwen_url.rstrip("/")

    def chat(self, base: str, payload: dict, timeout: int = 600) -> tuple[dict, dict]:
        body = json.dumps(payload).encode()
        req = urllib.request.Request(
            f"{base}/v1/chat/completions", data=body,
            headers={"Content-Type": "application/json"}, method="POST")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read())
        usage = data.get("usage") or {}
        return data, usage

    def apply_template(self, messages: list[dict]) -> str | None:
        body = json.dumps({"messages": messages}).encode()
        req = urllib.request.Request(
            f"{self.qwen}/apply-template", data=body,
            headers={"Content-Type": "application/json"}, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return json.loads(resp.read()).get("prompt")
        except (urllib.error.URLError, OSError, json.JSONDecodeError, KeyError):
            return None

    def completion_constrained(self, prompt: str, schema: dict,
                               max_tokens: int = 512) -> str | None:
        body = json.dumps({
            "prompt": prompt, "json_schema": schema,
            "n_predict": max_tokens, "temperature": 0.0, "cache_prompt": True,
        }).encode()
        req = urllib.request.Request(
            f"{self.qwen}/completion", data=body,
            headers={"Content-Type": "application/json"}, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=300) as resp:
                data = json.loads(resp.read())
            return data.get("content")
        except (urllib.error.URLError, OSError, json.JSONDecodeError):
            return None


def steer_verdict(up: Upstream, messages: list[dict], draft_message: dict) -> dict | None:
    """Ask Qwen whether the draft completes the conversation's task."""
    recent = messages[-6:]
    audit_msgs = [{"role": "user", "content": json.dumps({
        "conversation_tail": [
            {"role": m.get("role"), "content": (m.get("content") or "")[:1500],
             "tool_calls": bool(m.get("tool_calls"))}
            for m in recent],
        "draft_under_review": {
            "content": (draft_message.get("content") or "")[:2000],
            "has_tool_calls": has_tool_calls(draft_message)},
    }, indent=1)}]
    audit_prompt = (
        "You are the steering judge for a coding agent. The agent was working "
        "a task using tools. Review the conversation tail and its latest "
        "reply. Decide whether the TASK is complete, or name the concrete "
        "missing actions (e.g. 'edit src/x.rs', 'run cargo test').\n\n"
        + audit_msgs[0]["content"])
    out = up.completion_constrained(audit_prompt, STEER_SCHEMA)
    if out is None:
        return None
    try:
        obj = json.loads(out)
        if isinstance(obj.get("complete"), bool):
            return obj
    except json.JSONDecodeError:
        pass
    return None


def merge_reasoning(message: dict) -> dict:
    """If content is empty but reasoning_content exists (thinking model
    exhausted its budget mid-think), surface the reasoning so the client
    gets a non-blank turn."""
    if not (message.get("content") or "").strip():
        reason = message.get("reasoning_content") or ""
        if reason.strip():
            message["content"] = reason
    return message


def sse_from_response(data: dict) -> bytes:
    """Synthesize an SSE stream body from one complete chat response."""
    chunk = {
        "id": data.get("id", "shim"), "object": "chat.completion.chunk",
        "model": data.get("model", ""), "choices": [{
            "index": 0, "finish_reason": None,
            "delta": {"role": "assistant",
                      "content": (data["choices"][0]["message"].get("content") or "")},
        }],
    }
    tail = {"id": chunk["id"], "object": "chat.completion.chunk",
            "model": chunk["model"], "choices": [{"index": 0,
                                                  "finish_reason": "stop", "delta": {}}]}
    body = "".join(f"data: {json.dumps(c)}\n\n" for c in (chunk, tail)) + "data: [DONE]\n\n"
    return body.encode()


STATS = {"requests": 0, "steered_nudge": 0, "steered_override": 0,
         "passed_through": 0, "judged_complete": 0}


# ── Handler ──────────────────────────────────────────────────────────

class Shim(BaseHTTPRequestHandler):
    upstream: Upstream = Upstream("", "")  # replaced at server build
    mode: str = "steer"        # steer | passthrough
    steer_mode: str = "nudge"  # nudge | override
    verbose: bool = True

    def log_message(self, format, *args):  # noqa: A002 - stdlib signature
        pass

    def _note(self, msg: str):
        if self.verbose:
            print(f"[shim] {msg}", flush=True)

    def do_GET(self):
        if self.path.startswith(("/v1/models", "/health")):
            self._proxy_raw()
        else:
            self.send_error(404)

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            self._proxy_raw()
            return
        up = self.upstream
        length = int(self.headers.get("Content-Length", 0))
        payload = json.loads(self.rfile.read(length) or b"{}")
        wants_stream = bool(payload.get("stream"))
        payload["stream"] = False  # always buffer upstream
        # JetBrains' recommended sampling when the client doesn't care.
        payload.setdefault("temperature", 0.6)
        payload.setdefault("top_p", 0.95)
        payload.setdefault("top_k", 20)
        messages = payload.get("messages") or []
        key = session_key(messages)
        STATS["requests"] += 1

        data, usage = up.chat(up.mellum, payload)
        message = merge_reasoning(data["choices"][0]["message"])

        state = SESSIONS.setdefault(key, {})
        if self.mode != "steer":
            update_state(state, message)
            STATS["passed_through"] += 1
            self._respond(data, wants_stream)
            return

        flags = flag_turn(message, state)
        verdict = None
        if flags:
            self._note(f"session {key}: flags={flags} — judging")
            verdict = steer_verdict(up, messages, message)
            STATS["judged_complete"] += verdict is not None
        if verdict is None or verdict.get("complete"):
            update_state(state, message)
            self._respond(data, wants_stream)
            return

        missing = verdict.get("missing_actions") or []
        self._note(f"session {key}: INCOMPLETE — {len(missing)} missing action(s)")

        # Attempt 1: nudge the drafter with explicit orders.
        nudge_msgs = messages + [
            {"role": "assistant", "content": message.get("content") or ""},
            {"role": "user", "content": NUDGE_TEMPLATE.format(
                actions="\n".join(f"- {a}" for a in missing))},
        ]
        nudge_payload = dict(payload)
        nudge_payload["messages"] = nudge_msgs
        try:
            data2, _u2 = up.chat(up.mellum, nudge_payload)
        except (urllib.error.URLError, OSError, json.JSONDecodeError):
            data2 = None
        msg2 = merge_reasoning(data2["choices"][0]["message"]) if data2 else {}
        verdict2 = steer_verdict(self.upstream, nudge_msgs, msg2) if data2 else None

        if data2 and (verdict2 is None or verdict2.get("complete")):
            STATS["steered_nudge"] += 1
            update_state(state, msg2)
            self._respond(data2, wants_stream)
            return

        # Attempt 2: Qwen authors the tool calls outright.
        if self.steer_mode == "override" or not data2:
            ov_msgs = [{"role": "user", "content": json.dumps({
                "task_conversation_tail": [
                    {"role": m.get("role"),
                     "content": (m.get("content") or "")[:1200],
                     "tool_calls": bool(m.get("tool_calls"))}
                    for m in messages[-6:]],
                "missing_actions": missing,
                "instruction": "Author the next tool call(s) that perform the "
                               "missing actions. arguments_json must be a JSON "
                               "OBJECT serialized as a string.",
            }, indent=1)}]
            tmpl = up.apply_template(ov_msgs)
            out = up.completion_constrained(tmpl, OVERRIDE_SCHEMA,
                                                       max_tokens=1024) if tmpl else None
            if out:
                try:
                    obj = json.loads(out)
                    calls = []
                    for c in obj.get("tool_calls") or []:
                        args = c.get("arguments_json") or "{}"
                        parsed = json.loads(args) if isinstance(args, str) else args
                        calls.append({
                            "id": f"steer-{int(time.time()*1000)}-{len(calls)}",
                            "type": "function",
                            "function": {"name": c.get("name", ""),
                                         "arguments": json.dumps(parsed)},
                        })
                    if calls:
                        over = {"choices": [{"index": 0, "finish_reason": "tool_calls",
                                             "message": {
                                                 "role": "assistant",
                                                 "content": obj.get("explanation", ""),
                                                 "tool_calls": calls}}],
                                "model": data.get("model", ""),
                                "usage": usage}
                        STATS["steered_override"] += 1
                        update_state(state, over["choices"][0]["message"])
                        self._note(f"session {key}: OVERRIDE with {len(calls)} tool call(s)")
                        self._respond(over, wants_stream)
                        return
                except (json.JSONDecodeError, KeyError, TypeError):
                    pass

        # All attempts failed to produce complete work — ship the best we have.
        update_state(state, msg2 if data2 else message)
        self._respond(data2 if data2 else data, wants_stream)

    def _respond(self, data: dict, wants_stream: bool):
        body = sse_from_response(data) if wants_stream else json.dumps(data).encode()
        self.send_response(200)
        self.send_header("Content-Type",
                         "text/event-stream" if wants_stream else "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _proxy_raw(self):
        length = int(self.headers.get("Content-Length", 0))
        raw = self.rfile.read(length) if length else b""
        base = self.path
        url = f"{self.upstream.mellum}{base}"
        req = urllib.request.Request(url, data=raw or None, method=self.command)
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                body = resp.read()
                self.send_response(resp.status)
        except urllib.error.HTTPError as e:
            body = e.read()
            self.send_response(e.code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


SESSIONS: dict[str, dict] = {}


def main() -> int:
    p = argparse.ArgumentParser(description="REBIS steering shim")
    p.add_argument("--port", type=int, default=8090)
    p.add_argument("--mellum-url", default="http://127.0.0.1:8287")
    p.add_argument("--qwen-url", default="http://127.0.0.1:8279")
    p.add_argument("--mode", choices=["steer", "passthrough"], default="steer")
    p.add_argument("--steer-mode", choices=["nudge", "override"], default="nudge")
    p.add_argument("--selftest", action="store_true")
    args = p.parse_args()

    if args.selftest:
        return selftest()

    Shim.upstream = Upstream(args.mellum_url, args.qwen_url)
    Shim.mode = args.mode
    Shim.steer_mode = args.steer_mode
    srv = ThreadingHTTPServer(("127.0.0.1", args.port), Shim)
    print(f"[shim] :{args.port} → mellum={args.mellum_url} judge={args.qwen_url} "
          f"mode={args.mode}/{args.steer_mode}", flush=True)
    srv.serve_forever()
    return 0


def selftest() -> int:
    msgs = [{"role": "user", "content": "implement X in this repo"}]
    assert session_key(msgs) == session_key(list(reversed([])) + msgs)
    assert session_key(msgs) != session_key([{"role": "user", "content": "other"}])

    assert not has_tool_calls({"content": "hi"})
    assert has_tool_calls({"tool_calls": [{"id": "t"}]})
    assert looks_final({"content": "all done"})
    assert not looks_final({"tool_calls": []})

    st = {"saw_tool_call": True, "turn_count": 3}
    f = flag_turn({"content": "I think that's everything!"}, st)
    assert "no-tool-after-tools" in f
    st2 = {"turn_count": 1}
    assert "short-response" in flag_turn({"content": "ok"}, st2)
    assert not flag_turn({"content": "ok"}, {})  # first turn never flagged short
    st3 = {"last_content_hash": hashlib.sha256(b"same").hexdigest(),
           "turn_count": 5, "saw_tool_call": False}
    assert "repeated-content" in flag_turn({"content": "same"}, st3)

    upd = {}
    update_state(upd, {"content": "x"})
    assert upd["turn_count"] == 1
    update_state(upd, {"content": "y", "tool_calls": [1]})
    assert upd["saw_tool_call"] and upd["turn_count"] == 2

    body = sse_from_response({"choices": [{"message": {"content": "abc"}}], "model": "m"})
    assert b"data: [DONE]" in body and b"abc" in body

    print("selftest: all assertions passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
