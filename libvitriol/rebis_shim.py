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
import sys

_here = str(__import__("pathlib").Path(__file__).resolve().parent)
if _here not in sys.path:
    sys.path.insert(0, _here)
from rebis import shim_emit  # noqa: E402
import json
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# ── Steering verdict schema (Qwen-constrained) ───────────────────────

REQUEST_TIMEOUT = 600

REQUEST_TIMEOUT = 600


class UpstreamBadResponse(Exception):
    """Backend answered 200 with a body we cannot parse."""


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

def session_key(messages: list) -> str:
    """Conversation identity = hash of the first user message."""
    for m in messages:
        if not isinstance(m, dict):
            continue
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
        self.mellum = mellum_url.rstrip("/")   # Luna
        self.qwen = qwen_url.rstrip("/")       # Sol
        # head aliases used by gateway routing
        self.luna = self.mellum
        self.sol = self.qwen

    def chat(self, base: str, payload: dict, timeout: int = 600) -> tuple[dict, dict]:
        body = json.dumps(payload).encode()
        req = urllib.request.Request(
            f"{base}/v1/chat/completions", data=body,
            headers={"Content-Type": "application/json"}, method="POST")
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
        try:
            data = json.loads(raw)
        except json.JSONDecodeError as e:
            # Backends under memory pressure can emit empty/garbage 200
            # bodies — treat as an upstream failure, not a crash.
            raise UpstreamBadResponse(
                f"{base} returned non-JSON body ({len(raw)}B)") from e
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
                               max_tokens: int = 512,
                               temperature: float = 0.0) -> str | None:
        body = json.dumps({
            "prompt": prompt, "json_schema": schema,
            "n_predict": max_tokens, "temperature": temperature,
            "cache_prompt": True,
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


def mem_available_mib() -> int:
    try:
        for line in open("/proc/meminfo"):
            if line.startswith("MemAvailable:"):
                return int(line.split()[1]) // 1024
    except (OSError, ValueError, IndexError):
        pass
    return 1 << 30  # unknown: assume plenty


def health_up(url: str) -> bool:
    try:
        with urllib.request.urlopen(f"{url.rstrip('/')}/health", timeout=2) as r:
            return r.status == 200
    except (urllib.error.URLError, OSError):
        return False


def est_tokens(text: str) -> int:
    return max(1, len(text or "") // 4)


def access_line(route: str, head: str, t_entry: float, in_tok: int,
                out_tok: int | None, session: str,
                stream: str = "no", extra: str = "") -> str:
    dur = time.time() - t_entry
    out_s = str(out_tok) if out_tok is not None else "?"
    line = (f"[access] route={route} head={head} 200 {dur:.1f}s "
            f"in~{in_tok} out={out_tok} stream={stream} session={session[:8]}")
    if extra:
        line += f" {extra}"
    return line


def validate_tool_calls(message: dict) -> tuple[bool, str]:
    """Every tool_call's arguments must parse as JSON."""
    for tc in message.get("tool_calls") or []:
        args = (tc.get("function") or {}).get("arguments", "")
        if isinstance(args, str):
            try:
                json.loads(args)
            except json.JSONDecodeError:
                return False, "tool call arguments are not valid JSON"
    return True, ""


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


# ── Gateway compaction (day-long sessions) ───────────────────────────

COMPACT_THRESHOLD_TOKENS = 48000   # ~75% of the 65536 window
KEEP_RECENT_TOKENS = 10000         # active work never summarized
DIGEST_MARKER = "[SESSION MEMORY — compacted history]"


def estimate_tokens(text: str) -> int:
    return max(1, len(text) // 4)


def messages_tokens(messages: list[dict]) -> int:
    return sum(estimate_tokens(m.get("content") or "") for m in messages)


def needs_compaction(messages: list[dict],
                     threshold: int = COMPACT_THRESHOLD_TOKENS) -> bool:
    return messages_tokens(messages) > threshold


def split_for_compaction(messages: list[dict], keep_recent_tokens: int,
                         system_count: int):
    """Split into (head_system, old_to_summarize, recent_verbatim).

    head_system = leading system messages (never touched); recent span is
    filled back-to-front until KEEP_RECENT_TOKENS is exhausted.
    """
    if len(messages) <= system_count + 1:
        return messages[:system_count], [], messages[system_count:]
    head = messages[:system_count]
    body = messages[system_count:]
    keep: list[dict] = []
    budget = keep_recent_tokens
    i = len(body) - 1
    while i >= 0 and budget > 0:
        t = estimate_tokens(body[i].get("content") or "")
        keep.append(body[i])
        budget -= t
        i -= 1
    keep.reverse()
    old = body[:i + 1]
    return head, old, keep


COMPACT_INSTR = (
    "Summarize the following conversation fragment into a compact SESSION "
    "MEMORY digest for an engineering agent. Preserve: every file path "
    "mentioned, edit outcomes, command exit codes, decisions made, open "
    "questions, invariants stated. Compress prose aggressively. Output ONLY "
    "the digest as bullet lines.")


def sol_compact(up, messages_old: list[dict], prior_digest: str | None) -> str | None:
    """Sol writes/extends the session-memory digest. None on failure."""
    frag = "\n".join(
        f"[{m.get('role')}] {(m.get('content') or '')[:800]}"
        for m in messages_old[-40:])
    prior = (f"{DIGEST_MARKER} (previous):\n{prior_digest}\n\n"
             if prior_digest else "")
    payload = {
        "messages": [{"role": "user", "content":
                      f"{COMPACT_INSTR}\n\n{prior}"
                      f"# FRAGMENT TO ABSORB\n{frag}"}],
        "max_tokens": 2048, "temperature": 0.2,
        "chat_template_kwargs": {"enable_thinking": False},
    }
    try:
        data, _u = up.chat(up.sol, payload)
        text = (data["choices"][0]["message"].get("content") or "").strip()
        return text or None
    except Exception:  # noqa: BLE001 - compaction best-effort
        return None

# ── Gateway v2: routing + draft-audit pipeline ───────────────────────

DESIGN_MARKERS = ("why ", "how ", "architecture", "design", "explain",
                  "review", "plan", "tradeoff", "compare", "should we")

def estimate_reason_need(text) -> bool:
    low = (text or "").lower()
    if len(low) > 1500:
        return True
    head = low[:600]
    return any(mk in head for mk in DESIGN_MARKERS)


def classify_turn(messages: list[dict], tools_attached: bool,
                  forced: str | None = None) -> str:
    """Route decision: 'reason' (Sol/Qwen) | 'draft' (Luna/Mellum)
    | 'pipeline' (Luna drafts, Sol audits).

    Ladder (documented in plans/rebis-gateway-v2):
      1. explicit escape hatch wins
      2. tools attached -> pipeline (Luna drafts; audit intensity scales:
         kickoff + finals get full Sol verdicts, executor continuations
         schema-only)
      3. no-tools chat -> reason (Sol; bare-chat Luna drafting degenerates)
      4. fallback -> reason (safe default: quality over speed)
    """
    if forced in ("rebis-qwen",):
        return "reason"
    if forced in ("rebis-mellum",):
        return "draft"
    if not messages:
        return "reason"
    has_calls = any(isinstance(m, dict) and m.get("tool_calls")
                    for m in messages
                    if isinstance(m, dict) and m.get("role") == "assistant")
    last = messages[-1] if isinstance(messages[-1], dict) else {}
    last_role = last.get("role")
    # Luna-first: ALL agentic turns draft on Luna; Sol verifies via the
    # pipeline's audit layer (intensity scales by turn position).
    if tools_attached:
        return "pipeline"
    # No toolset => not an agentic execution turn. Quality-first: Sol.
    # (Bare-chat Luna drafts degenerate without harness structure.)
    return "reason"


def synthesize_models(gateway_id: str = "rebis") -> dict:
    """OpenAI-shaped /v1/models advertising the trenchcoat + heads."""
    def entry(mid, name):
        return {"id": mid, "object": "model", "owned_by": "rebis",
                "name": name}
    return {"object": "list", "data": [
        entry(gateway_id, "REBIS — Sol+Luna unified"),
        entry("rebis-qwen", "Sol head (Qwen3.8, reasoning)"),
        entry("rebis-mellum", "Luna head (Mellum2, drafting)"),
    ]}


def parse_sse_stream(resp) -> tuple[dict, dict]:
    """Consume an OpenAI-compatible SSE body into one assistant message.

    Returns (message, usage). Tool-call deltas are stitched minimally
    (index-keyed accumulation of name/arguments fragments).
    """
    content_parts: list[str] = []
    calls: dict[int, dict] = {}
    usage: dict = {}
    finish = None
    for raw in resp:
        line = raw.decode(errors="ignore").strip() if isinstance(raw, bytes)             else raw.strip()
        if not line.startswith("data:"):
            continue
        data = line[5:].strip()
        if data == "[DONE]":
            break
        try:
            obj = json.loads(data)
        except json.JSONDecodeError:
            continue
        if obj.get("usage"):
            usage = obj["usage"]
        for ch in obj.get("choices") or []:
            if ch.get("finish_reason"):
                finish = ch["finish_reason"]
            delta = ch.get("delta") or {}
            if delta.get("content"):
                content_parts.append(delta["content"])
            for tc in delta.get("tool_calls") or []:
                idx = tc.get("index", 0)
                slot = calls.setdefault(idx, {"id": "", "type": "function",
                                              "function": {"name": "",
                                                           "arguments": ""}})
                if tc.get("id"):
                    slot["id"] = tc["id"]
                fn = tc.get("function") or {}
                if fn.get("name"):
                    slot["function"]["name"] += fn["name"]
                if fn.get("arguments"):
                    slot["function"]["arguments"] += fn["arguments"]
    message: dict = {"role": "assistant", "content": "".join(content_parts)}
    if calls:
        message["tool_calls"] = [calls[i] for i in sorted(calls)]
    if finish:
        message["finish_reason"] = finish
    return message, usage

DISTILL_DIR = "/home/randozart/.vitriol/distill"

STATS = {"requests": 0, "steered_nudge": 0, "steered_override": 0,
         "passed_through": 0, "judged_complete": 0}


# ── Handler ──────────────────────────────────────────────────────────

class Shim(BaseHTTPRequestHandler):
    upstream: Upstream = Upstream("", "")
    distill_dir: str = "/home/randozart/.vitriol/distill"  # replaced at server build
    mode: str = "steer"        # gateway | steer | passthrough
    steer_mode: str = "nudge"  # nudge | override
    luna_model: str = ""
    sol_model: str = ""
    compact: bool = True
    verbose: bool = True

    def log_message(self, format, *args):  # noqa: A002 - stdlib signature
        pass

    def _note(self, msg: str):
        if self.verbose:
            print(f"[shim] {msg}", flush=True)

    def do_GET(self):
        if self.mode == "gateway" and self.path.startswith("/v1/models"):
            body = json.dumps(synthesize_models()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path.startswith(("/v1/models", "/health")):
            self._proxy_raw()
        else:
            self.send_error(404)

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            self._proxy_raw()
            return
        if self.mode == "gateway":
            length = int(self.headers.get("Content-Length", 0))
            try:
                payload = json.loads(self.rfile.read(length) or b"{}")
            except json.JSONDecodeError as e:
                self._note(f"unparseable request body: {e}")
                self.send_error(400, "invalid JSON body")
                return
            if not isinstance(payload, dict):
                self.send_error(400, "request body must be a JSON object")
                return
            wants_stream = bool(payload.get("stream"))
            messages = payload.get("messages")
            if not isinstance(messages, list):
                self.send_error(400, "messages must be a list")
                return
            key = session_key(messages)
            STATS["requests"] += 1
            # Memory guardrail: refuse to route when the box is starving —
            # a hard freeze takes everything down, this takes down one turn.
            if mem_available_mib() < 1200:
                body = json.dumps({"error": {
                    "message": "REBIS: host memory pressure (MemAvailable "
                               "< 1200 MiB). Close builds/other workloads "
                               "or wait — protective backpressure.",
                    "type": "memory_pressure"}}).encode()
                self._note(f"memory guardrail: {mem_available_mib()} MiB free — 503")
                try:
                    self.send_response(503)
                    self.send_header("Retry-After", "60")
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass
                return
            for attempt in (1, 2):
                try:
                    self.gateway_turn(payload, key, wants_stream)
                    return
                except (BrokenPipeError, ConnectionResetError):
                    self._note("client disconnected mid-turn")
                    return
                except Exception as e:
                    # Catch-all: ANY handler crash must become a clean 503,
                    # never a silent connection close (hermes reads that as
                    # a timeout and drops the session).
                    refused = isinstance(e, (urllib.error.URLError, OSError))
                    self._note(f"gateway error (attempt {attempt}): {type(e).__name__}: {e}")
                    if refused and attempt == 1:
                        time.sleep(3)
                        continue
                    body = json.dumps({"error": {
                        "message": f"REBIS gateway error: {type(e).__name__}: "
                                   f"{str(e)[:200]}. Retry shortly.",
                        "type": "gateway_error"}}).encode()
                    try:
                        self.send_response(502)
                        self.send_header("Content-Type", "application/json")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                    except (BrokenPipeError, ConnectionResetError):
                        self._note("client gone during 502")
                    return
                    body = json.dumps({"error": {
                        "message": "REBIS heads are respawning — retry in "
                                   "~30s; the supervisor restores them.",
                        "type": "backend_unavailable"}}).encode()
                    try:
                        self.send_response(503)
                        self.send_header("Retry-After", "30")
                        self.send_header("Content-Type", "application/json")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                    except (BrokenPipeError, ConnectionResetError):
                        self._note("client gone during 503")
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
            if verdict is not None:
                shim_emit(self.distill_dir, {
                    "type": "shim_judged", "session": key,
                    "flags": flags,
                    "complete": bool(verdict.get("complete")),
                    "missing_actions": verdict.get("missing_actions") or [],
                    "draft_content": (message.get("content") or "")[:4000],
                })
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
            shim_emit(self.distill_dir, {
                "type": "steer_nudge", "session": key,
                "original_response": (message.get("content") or "")[:4000],
                "final_response": (msg2.get("content") or "")[:4000],
            })
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
                        shim_emit(self.distill_dir, {
                            "type": "steer_override", "session": key,
                            "override_calls": calls,
                        })
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
        try:
            self.send_response(200)
            self.send_header("Content-Type",
                             "text/event-stream" if wants_stream else "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            # Client gave up (steering latency can exceed its timeout) —
            # nothing to send; keep the worker alive for the next request.
            self._note("client disconnected before response")

    def system_count(self, messages: list[dict]) -> int:
        n = 0
        for m in messages:
            if m.get("role") == "system":
                n += 1
            else:
                break
        return n

    def gateway_turn(self, payload: dict, key: str, wants_stream: bool):
        """REBIS gateway: route to a head or run the draft-audit pipeline."""
        t_entry = time.time()
        messages = payload.get("messages") or []
        in_tok = est_tokens(" ".join(
            (m.get("content") or "") for m in messages[-3:]))
        # Day-long sessions: compact before routing when history outgrows window.
        if self.compact and needs_compaction(messages):
            head, old_msgs, recent = split_for_compaction(
                messages, KEEP_RECENT_TOKENS, self.system_count(messages))
            prior = next((m.get("content", "") for m in reversed(head)
                          if DIGEST_MARKER in (m.get("content") or "")), None)
            digest = sol_compact(self.upstream, old_msgs, prior)
            if digest and old_msgs:
                digest_msg = {"role": "system",
                              "content": f"{DIGEST_MARKER}\n{digest}"}
                messages = head + [digest_msg] + recent
                payload["messages"] = messages
                shim_emit(self.distill_dir, {
                    "type": "compaction", "session": key,
                    "summarized_turns": len(old_msgs),
                    "summarized_tokens": messages_tokens(old_msgs),
                    "kept_recent": len(recent),
                })
                self._note(f"session {key}: compacted "
                           f"{len(old_msgs)} turns -> digest")
        up = self.upstream
        model_id = (payload.get("model") or "").strip()
        forced = model_id if model_id.startswith("rebis-") else None
        tools_attached = bool(payload.get("tools"))
        messages = payload.get("messages") or []
        route = classify_turn(messages, tools_attached, forced)
        mt_req = payload.get("max_tokens") or 0
        last_user = next((m for m in reversed(messages)
                          if m.get("role") == "user"), None)
        if (route == "reason" and not tools_attached and 0 < mt_req <= 400
                and last_user is not None
                and len(last_user.get("content") or "") <= 600):
            route = "draft"
            self._note(f"session {key}: aux fast-path (max_tokens {mt_req})")
        self._note(f"session {key}: route={route}"
                   + (f" (forced={forced})" if forced else ""))

        def ship(data: dict):
            drec = {"type": "gateway_turn", "session": key,
                    "route": route, "model": model_id,
                    "usage": data.get("usage") or {}}
            shim_emit(self.distill_dir, drec)
            self._respond(data, wants_stream)

        if route == "draft":
            # Luna fast path — with malformed-tool-JSON recovery.
            luna_payload = dict(payload)
            luna_payload["model"] = self.luna_model
            try:
                data, _u = up.chat(up.mellum, luna_payload)
            except (urllib.error.HTTPError, UpstreamBadResponse) as e:
                # Luna's known failures: malformed tool-call JSON (5xx) or
                # garbage 200 bodies under memory pressure. One corrective
                # retry, then Sol covers the turn.
                self._note(f"luna draft failed ({type(e).__name__}) — "
                           "retrying, then Sol fallback")
                retry_payload = dict(luna_payload)
                retry_payload["messages"] = list(luna_payload["messages"]) + [
                    {"role": "assistant",
                     "content": "[my previous reply failed to format "
                                "properly]"},
                    {"role": "user",
                     "content": "Re-issue your last reply correctly. If it "
                                "contained a tool call, use VALID JSON "
                                "arguments per the schema."},
                ]
                data, _u = up.chat(up.mellum, retry_payload)
            message = merge_reasoning(data["choices"][0]["message"])
            update_state(SESSIONS.setdefault(key, {}), message)
            self._note(access_line("draft", "luna", t_entry, in_tok,
                                   (_u or {}).get("completion_tokens"), key))
            ship(data)
            return

        if route == "reason":
            # Sol untouched — full depth reasoning (budget-capped server-side).
            sol_payload = dict(payload)
            sol_payload["model"] = self.sol_model
            # Respect the client's cap — forcing a high floor turned agent
            # turns into multi-minute marathons.
            sol_payload["max_tokens"] = sol_payload.get("max_tokens") or 4096
            sol_payload.setdefault("chat_template_kwargs",
                                   {"enable_thinking": True})

            if wants_stream:
                # LIVE RELAY: stream Sol's tokens straight through so the
                # connection never idles — long thinking turns cannot hit
                # client timeouts.
                sol_payload["stream"] = True
                sbody = json.dumps(sol_payload).encode()
                sreq = urllib.request.Request(
                    f"{up.sol}/v1/chat/completions", data=sbody,
                    headers={"Content-Type": "application/json"},
                    method="POST")
                try:
                    sresp = urllib.request.urlopen(sreq, timeout=REQUEST_TIMEOUT)
                except (urllib.error.URLError, OSError) as e:
                    self._note(f"sol stream error: {e}")
                    self.send_error(502, str(e)[:200])
                    return
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.end_headers()
                content_parts: list[str] = []
                try:
                    for raw in sresp:
                        line = raw.decode(errors="ignore").strip()
                        if not line.startswith("data:"):
                            continue
                        chunk = line[5:].strip()
                        if chunk == "[DONE]":
                            self.wfile.write(b"data: [DONE]\n\n")
                            break
                        try:
                            obj = json.loads(chunk)
                        except json.JSONDecodeError:
                            continue
                        for ch in obj.get("choices") or []:
                            piece = (ch.get("delta") or {}).get("content")
                            if piece:
                                content_parts.append(piece)
                        self.wfile.write(f"data: {chunk}\n\n".encode())
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass
                final = {"role": "assistant",
                         "content": "".join(content_parts)}
                update_state(SESSIONS.setdefault(key, {}),
                             merge_reasoning(final))
                self._note(access_line("reason", "sol", t_entry, in_tok,
                                       len("".join(content_parts)) // 4, key,
                                       stream="yes"))
                shim_emit(self.distill_dir, {
                    "type": "gateway_turn", "session": key,
                    "route": "reason-streamed",
                    "content_len": len("".join(content_parts)),
                })
                return

            data, _u = up.chat(up.sol, sol_payload)
            message = merge_reasoning(data["choices"][0]["message"])
            update_state(SESSIONS.setdefault(key, {}), message)
            self._note(access_line("reason", "sol", t_entry, in_tok,
                                   (data.get("usage") or {}).get("completion_tokens"),
                                   key))
            ship(data)
            return

        # ── pipeline: Luna drafts while Sol ingests; Sol audits ──
        luna_payload = dict(payload)
        luna_payload["model"] = self.luna_model
        luna_payload["stream"] = True  # incremental consumption
        luna_payload["max_tokens"] = max(luna_payload.get("max_tokens") or 0,
                                         4096)
        luna_payload.setdefault("temperature", 0.6)
        luna_payload.setdefault("top_p", 0.95)
        luna_payload.setdefault("top_k", 20)
        luna_payload["chat_template_kwargs"] = {"enable_thinking": False}

        body = json.dumps(luna_payload).encode()
        req = urllib.request.Request(
            f"{up.mellum}/v1/chat/completions", data=body,
            headers={"Content-Type": "application/json"}, method="POST")
        t0 = time.time()
        warm_len = 0
        buf_text: list[str] = []
        last_warm = [0.0]
        audit_tail_msgs = messages[-4:]

        def build_audit_prompt(candidate: str) -> str:
            return json.dumps({
                "conversation_tail": [
                    {"role": m.get("role"),
                     "content": (m.get("content") or "")[:1200],
                     "tool_calls": bool(m.get("tool_calls"))}
                    for m in audit_tail_msgs],
                "candidate_response": candidate[:6000],
            }, indent=1)

        def warm_sol(candidate_so_far: str):
            prompt = ("PREFILL WARMING for an upcoming audit. Reply with "
                      "only the word ok.\n\n" + build_audit_prompt(candidate_so_far))
            try:
                b2 = json.dumps({"prompt": prompt, "n_predict": 1,
                                 "temperature": 0.0,
                                 "cache_prompt": True}).encode()
                r2 = urllib.request.Request(
                    f"{up.sol}/completion", data=b2,
                    headers={"Content-Type": "application/json"},
                    method="POST")
                urllib.request.urlopen(r2, timeout=120).read()
            except (urllib.error.URLError, OSError):
                pass

        with urllib.request.urlopen(req, timeout=REQUEST_TIMEOUT) as resp:
            # parse_sse_stream stitches content AND tool_calls; tool-call
            # turns are Luna acting (valid mid-flight work), not finals.
            parsed_msg, parsed_usage = parse_sse_stream(resp)
            buf_text.append(parsed_msg.get("content") or "")
        grown = sum(len(t) for t in buf_text)
        if grown - warm_len >= 1024:
            warm_sol("".join(buf_text))
        draft_message = dict(parsed_msg)
        if not draft_message.get("content"):
            draft_message["content"] = ""
        draft_time = round(time.time() - t0, 2)

        flags = flag_turn(draft_message, state := SESSIONS.setdefault(key, {}))
        schema_ok, schema_report = validate_tool_calls(draft_message)
        first_assistant = not any(
            m.get("role") == "assistant" for m in messages[:-1])

        # Executor continuation with well-formed tool calls: schema-only,
        # ship without a Sol round-trip.
        if (has_tool_calls(draft_message) and schema_ok
                and not flags and not first_assistant):
            update_state(state, draft_message)
            self._note(access_line("pipeline", "luna", t_entry, in_tok,
                                   est_tokens(draft_message.get("content")), key,
                                   extra="audit=schema-only"))
            shim_emit(self.distill_dir, {
                "type": "pipeline_pass", "session": key,
                "audited": False, "schema_only": True,
                "draft": draft_message.get("content", "")[:4000]})
            self._respond({"choices": [{"index": 0, "finish_reason":
                                        draft_message.get("finish_reason", "stop"),
                                        "message": draft_message}],
                           "model": "rebis"}, wants_stream)
            return

        # Malformed tool-call arguments are a Luna capability limit, not an
        # attention slip — nudging rarely recovers them (measured). Skip
        # straight to the audit; Sol correction authors valid calls.
        if has_tool_calls(draft_message) and not schema_ok:
            self._note(f"session {key}: malformed tool args — escalating to Sol")

        # Sol ingested the growing draft already — final warm covers the tail.
        warm_sol(draft_message.get("content") or "")
        candidate = build_audit_prompt(draft_message.get("content") or "")
        audit_prompt = (
            "You are the steering judge inside REBIS, a dual-model agent. "
            "Luna (fast drafter) produced the CANDIDATE RESPONSE for the "
            "conversation tail. Decide whether it adequately serves the "
            "task: correct tool usage, no hallucinated tools or APIs, task "
            "actually advanced. Reply ONLY with JSON:\n"
            '{"complete": <bool>, "reasoning": "<brief>", '
            '"missing_actions": ["..."], '
            '"corrected_response": "<full replacement text if incomplete>" }\n\n'
            + candidate)
        out = up.completion_constrained(audit_prompt, STEER_SCHEMA,
                                        max_tokens=2048, temperature=0.0)
        verdict = None
        if out:
            try:
                vj = json.loads(out)
                if isinstance(vj.get("complete"), bool):
                    verdict = vj
            except json.JSONDecodeError:
                pass
        STATS["judged_complete"] += verdict is not None
        sol_reachable = health_up(up.sol)
        shim_emit(self.distill_dir, {
            "type": "pipeline_audited", "session": key, "flags": flags,
            "complete": bool(verdict and verdict.get("complete")),
            "sol_down": verdict is None and not sol_reachable,
            "missing_actions": (verdict or {}).get("missing_actions") or [],
            "draft_content": (draft_message.get("content") or "")[:4000],
        })

        if verdict is None:
            # Sol unreachable or unparseable: availability-first — ship the
            # draft unaudited, marked for review.
            update_state(state, draft_message)
            self._note(access_line("pipeline", "luna", t_entry, in_tok,
                                   est_tokens(draft_message.get("content")), key,
                                   extra="audit=UNAUDITED(sol_down)"
                                         if not sol_reachable
                                         else "audit=UNAUDITED(unparseable)"))
            shim_emit(self.distill_dir, {
                "type": "pipeline_unaudited", "session": key,
                "reason": "sol_down" if not sol_reachable else "unparseable",
                "draft": draft_message.get("content", "")[:4000]})
            self._respond({"choices": [{"index": 0, "finish_reason":
                                        draft_message.get("finish_reason", "stop"),
                                        "message": draft_message}],
                           "model": "rebis"}, wants_stream)
            return

        if verdict.get("complete"):
            update_state(state, draft_message)
            self._note(access_line("pipeline", "luna+sol", t_entry, in_tok,
                                   est_tokens(draft_message.get("content")), key,
                                   extra="audit=pass"))
            shim_emit(self.distill_dir, {
                "type": "pipeline_pass", "session": key,
                "audited": True, "draft": draft_message.get("content", "")[:4000]})
            self._respond({"choices": [{"index": 0, "finish_reason":
                                        draft_message.get("finish_reason", "stop"),
                                        "message": draft_message}],
                           "model": "rebis"}, wants_stream)
            return

        missing = verdict.get("missing_actions") or []
        corrected = (verdict.get("corrected_response") or "").strip()
        self._note(f"session {key}: AUDIT FAILED — {len(missing)} missing; "
                   f"correcting via Sol")

        # Sol authors the correction natively (proper tool_calls format).
        corr_msgs = list(messages) + [
            {"role": "assistant", "content": draft_message.get("content") or ""},
            {"role": "user", "content":
             "Your previous reply was judged INCOMPLETE/INCORRECT by review.\n"
             "Missing actions:\n" +
             "\n".join(f"- {a}" for a in missing) +
             "\nProduce the corrected full reply now, using the available "
             "tools properly."}]
        corr_payload = dict(payload)
        corr_payload["messages"] = corr_msgs
        corr_payload["model"] = self.sol_model
        corr_payload.setdefault("chat_template_kwargs",
                                {"enable_thinking": False})
        corr_data, _u3 = up.chat(up.sol, corr_payload)
        final_msg = merge_reasoning(corr_data["choices"][0]["message"])
        STATS["steered_nudge"] += 1
        shim_emit(self.distill_dir, {
            "type": "steer_correct", "session": key,
            "original_response": (draft_message.get("content") or "")[:4000],
            "final_response": (final_msg.get("content") or "")[:4000],
            "missing_actions": missing,
        })
        update_state(state, final_msg)
        self._note(access_line("pipeline", "luna+sol", t_entry, in_tok,
                               est_tokens(final_msg.get("content")), key,
                               extra="audit=corrected"))
        shim_emit(self.distill_dir, {
            "type": "steer_correct", "session": key,
            "original_response": (draft_message.get("content") or "")[:4000],
            "final_response": (final_msg.get("content") or "")[:4000],
            "missing_actions": missing,
        })
        self._respond({"choices": [{"index": 0, "finish_reason":
                                    final_msg.get("finish_reason", "stop"),
                                    "message": final_msg}],
                       "model": "rebis"}, wants_stream)

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
    p.add_argument("--port", type=int, default=8280)  # Mercury, Hg=80
    p.add_argument("--mellum-url", "--luna-url",
                   dest="mellum_url", default="http://127.0.0.1:8247")
    p.add_argument("--qwen-url", "--sol-url",
                   dest="qwen_url", default="http://127.0.0.1:8279")
    p.add_argument("--luna-model",
                   default="Mellum2-12B-A2.5B-Thinking.i1-IQ4_XS.gguf")
    p.add_argument("--sol-model",
                   default="Qwen3.8-27B-UD-IQ2_S.gguf")
    p.add_argument("--distill-dir", default=DISTILL_DIR)
    p.add_argument("--mode", choices=["gateway", "steer", "passthrough"],
                   default="gateway")
    p.add_argument("--steer-mode", choices=["nudge", "override"], default="nudge")
    p.add_argument("--selftest", action="store_true")
    args = p.parse_args()

    if args.selftest:
        return selftest()

    Shim.upstream = Upstream(args.mellum_url, args.qwen_url)
    Shim.distill_dir = args.distill_dir
    Shim.luna_model = args.luna_model
    Shim.sol_model = args.sol_model
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

    # Gateway routing ladder
    kickoff = [{"role": "user", "content": "implement X using the tools"}]
    assert classify_turn(kickoff, True) == "pipeline"
    exec_hist = kickoff + [
        {"role": "assistant", "tool_calls": [{"id": "1"}]},
        {"role": "tool", "content": "result"},
    ]
    assert classify_turn(exec_hist, True) == "pipeline"
    finalizing = kickoff + [
        {"role": "assistant", "tool_calls": [{"id": "1"}]},
        {"role": "tool", "content": "result"},
        {"role": "assistant", "content": "All done, here is the summary."},
    ]
    assert classify_turn(finalizing, True) == "pipeline"
    # No-tools turns are quality-first: always Sol.
    assert classify_turn([{"role": "user",
                           "content": "fix typo in README"}], False) == "reason"
    assert classify_turn([{"role": "user",
                           "content": "Explain architecture tradeoffs. " * 5}],
                          False) == "reason"
    assert classify_turn([{"role": "user", "content": "?"}],
                         True, forced="rebis-qwen") == "reason"
    assert classify_turn([{"role": "user", "content": "?"}],
                         True, forced="rebis-mellum") == "draft"

    models = synthesize_models()
    ids = [m["id"] for m in models["data"]]
    assert ids == ["rebis", "rebis-qwen", "rebis-mellum"]

    msg, _u = parse_sse_stream(iter([
        b'data: {"choices":[{"delta":{"role":"assistant"}}]}\n',
        b'data: {"choices":[{"delta":{"content":"he"}}]}\n',
        b'data: {"choices":[{"delta":{"content":"llo"}}]}\n',
        b'data: {"choices":[{"finish_reason":"stop","delta":{}}],'
        b'"usage":{"total_tokens":9}}\n',
        b"data: [DONE]\n",
    ]))
    assert msg["content"] == "hello"
    assert msg["finish_reason"] == "stop" and _u["total_tokens"] == 9

    # Compaction split: head preserved, old summarized, recent verbatim
    msgs = ([{"role": "system", "content": "sys"}] +
            [{"role": "user" if i % 2 == 0 else "assistant",
              "content": "x" * 2000} for i in range(10)])
    head, oldm, keep = split_for_compaction(msgs, 1200, 1)
    assert len(head) == 1 and head[0]["role"] == "system"
    assert len(oldm) > 0 and len(keep) > 0
    assert oldm + keep == msgs[1:] or (oldm + keep) == msgs[1:]
    assert messages_tokens(keep) <= 1200 + max(
        estimate_tokens(m["content"]) for m in keep)
    assert needs_compaction(msgs, threshold=1000)
    assert not needs_compaction(msgs, threshold=10**9)

    print("selftest: all assertions passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
