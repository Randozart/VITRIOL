#!/usr/bin/env python3
"""REBIS — dual-model drafter/verifier refinement loop for VITRIOL.

Mellum2 (fast MoE drafter, pinned GPU) produces code drafts from a Mandatum
packet; Qwen3.8 (slow reasoner) audits every draft against the stated
invariants with per-invariant evidence, and returns surgical delta orders. A
compiler gate provides the objective signal; the verifier provides the
semantic one. Hard iteration cap and wall-clock budget prevent runaway loops.

Agentic durability (v2): JSONL journal + --resume, per-call retry, server
auto-respawn, token accounting, JSON report.

Protocol names: packet = Mandatum, loop = Rebis, shadow prefill = Anticipatio.
"""

import argparse
import json
import re
import shlex
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone
from pathlib import Path

MAX_ITERATIONS = 3
COMPILE_TIMEOUT = 180
REQUEST_TIMEOUT = 600
CHAT_RETRIES = 1
SERVER_START_TIMEOUT = 900
DEFAULT_JOURNAL_DIR = "/tmp/opencode/rebis-journal"
DEFAULT_DISTILL_DIR = str(Path.home() / ".vitriol" / "distill")


# ── Distillation capture ─────────────────────────────────────────────

def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


class DistillRecorder:
    """Append-per-event training-data capture for one Rebis run.

    Records embed repo code by design — the distill directory is local-only
    and must never be committed or synced. Poked/rejected iterations are kept
    with the same fidelity as accepted ones: they are the rejected side of
    preference pairs.
    """

    def __init__(self, distill_dir: str, task_id: str, enabled: bool = True):
        self.enabled = enabled
        self.path = Path(distill_dir) / f"{task_id}.jsonl"

    def emit(self, etype: str, payload: dict | None = None) -> None:
        if not self.enabled:
            return
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            record = {"ts": _now_iso(), "type": etype, **(payload or {})}
            with open(self.path, "a") as f:
                f.write(json.dumps(record) + "\n")
        except OSError as e:
            print(f"[rebis] distill write failed: {e}", file=sys.stderr)

    def snapshot_paths(self, workdir: str, paths) -> dict:
        """files_before/files_after helper: {rel: content} for existing paths."""
        out = {}
        for rel in paths:
            p = Path(workdir) / rel
            if p.exists():
                try:
                    out[rel] = p.read_text()
                except OSError:
                    pass
        return out


def shim_emit(distill_dir: str, record: dict) -> None:
    """Shim-side capture into the shared distill store."""
    try:
        d = Path(distill_dir)
        d.mkdir(parents=True, exist_ok=True)
        with open(d / "shim-events.jsonl", "a") as f:
            f.write(json.dumps({"ts": _now_iso(), **record}) + "\n")
    except OSError:
        pass


# ── Mandatum packet ──────────────────────────────────────────────────

@dataclass
class FileSlice:
    path: str = ""
    start: int = 0
    end: int = 0
    content: str = ""


@dataclass
class Mandatum:
    """The order packet passed from planner to drafter.

    Layout rule: stable blocks (objective, invariants, constraints,
    contract) come FIRST so repeated turns hit the server prefix cache;
    volatile file slices come LAST. Multi-file tasks list several slices;
    the drafter must emit each file under a `### <path>` header.
    """
    objective: str = ""
    invariants: list[str] = field(default_factory=list)
    constraints: list[str] = field(default_factory=list)
    output_contract: str = ""
    file_slices: list[FileSlice] = field(default_factory=list)
    # Loop plumbing (not sent to the model)
    workdir: str = "."
    compile_cmd: str = ""
    max_iterations: int = MAX_ITERATIONS
    task_id: str = ""
    # draft_mode:
    #   "file"    — complete file contents per ### header. Only viable for
    #               files small enough to re-emit within the token budget
    #               (empirically ~250 lines for Mellum-Thinking at 4096).
    #   "patch"   — one unified diff via `git apply`. Requires verbatim hunk
    #               context; models miscount/hallucinate it often.
    #   "replace" — SEARCH/REPLACE blocks per ### header, exact-match applied
    #               against the live file (aider-style). Most reliable delta
    #               protocol observed for this drafter class.
    draft_mode: str = "file"
    # "compiler_only" — success = clean apply + green gate; skips the LLM
    #   auditor entirely (right when every invariant is enforced by the gate:
   #    test-emitting invariants, greppable structure)
    # "llm"           — Qwen audits every draft (semantic invariants)
    verify_mode: str = "llm"
    # 0 = auto by draft_mode (file→4096, patch→8192); else explicit cap.
    draft_budget: int = 0

    @classmethod
    def load(cls, path: str) -> "Mandatum":
        raw = json.loads(Path(path).read_text())
        slices = raw.pop("file_slices", None)
        if slices is None:
            legacy = raw.pop("file_slice", None)
            slices = [legacy] if legacy else []
        raw["file_slices"] = [FileSlice(**fs) for fs in slices]
        # Drop keys from older packet versions (e.g. v1's out_path) instead
        # of crashing — forward compatibility for stored task files.
        known = set(cls.__dataclass_fields__)
        dropped = set(raw) - known
        if dropped:
            print(f"[rebis] ignoring unknown task keys: {sorted(dropped)}")
        raw = {k: v for k, v in raw.items() if k in known}
        m = cls(**raw)
        if not m.task_id:
            m.task_id = Path(path).stem + "-" + datetime.now(timezone.utc).strftime("%H%M%S")
        return m

    def stable_prefix(self) -> str:
        """Everything invariant across loop turns — the cacheable head."""
        lines = [f"# OBJECTIVE\n{self.objective}", "# INVARIANTS"]
        lines += [f"- {i}" for i in self.invariants]
        if self.constraints:
            lines.append("# CONSTRAINTS")
            lines += [f"- {c}" for c in self.constraints]
        lines.append(f"# OUTPUT CONTRACT\n{self.output_contract}")
        return "\n".join(lines)

    def volatile_body(self, delta: list[str] | None = None) -> str:
        """Per-turn tail: current file slices plus any correction orders."""
        parts = []
        for fs in self.file_slices:
            parts.append(
                f"# FILE {fs.path} (lines {fs.start}-{fs.end})\n"
                f"```{lang_of(fs.path)}\n{fs.content}\n```")
        if delta:
            orders = "\n".join(f"{n+1}. {d}" for n, d in enumerate(delta))
            parts.append(f"# CORRECTION ORDERS (apply exactly these)\n{orders}")
        return "\n\n".join(parts)

    def drafter_messages(self, delta: list[str] | None = None,
                         current_files: dict[str, str] | None = None) -> list[dict]:
        """Build the drafter prompt.

        On correction turns `current_files` carries the LAST draft so the
        model patches its own prior work instead of restarting from the
        original slice — regeneration-from-scratch loses converged progress.
        In patch mode the output contract is a single unified diff instead.
        """
        parts = []
        for fs in self.file_slices:
            content = (current_files or {}).get(fs.path) or fs.content
            label = "CURRENT STATE" if (current_files and fs.path in current_files) \
                else f"lines {fs.start}-{fs.end}"
            parts.append(
                f"# FILE {fs.path} ({label})\n"
                f"```{lang_of(fs.path)}\n{content}\n```")
        if delta:
            orders = "\n".join(f"{n+1}. {d}" for n, d in enumerate(delta))
            parts.append(
                "# CORRECTION ORDERS (apply exactly these to the CURRENT STATE;\n"
                "keep everything else intact)\n" + orders)
        head = f"{self.stable_prefix()}\n\n" + "\n\n".join(parts)
        if self.draft_mode == "patch":
            fmt = ("\n\n# OUTPUT FORMAT\n"
                   "Do NOT ask questions. Output ONLY the diff.\n"
                   "The CURRENT STATE above is the complete live file on disk.\n"
                   "Emit ONE unified diff (`git apply` format) whose context "
                   "lines are copied VERBATIM from CURRENT STATE, with correct "
                   "@@ line numbers, that transforms it into the required "
                   "state. Modify the existing file — never emit a creation "
                   "diff:\n"
                   "```diff\n<the full diff>\n```")
        elif self.draft_mode == "replace":
            fmt = ("\n\n# OUTPUT FORMAT\n"
                   "Do NOT ask questions. Emit SEARCH/REPLACE blocks.\n"
                   "For each change, under a `### <path>` header:\n"
                   "<<<<<<< SEARCH\n"
                   "<exact lines copied VERBATIM from CURRENT STATE>\n"
                   "=======\n"
                   "<replacement lines>\n"
                   ">>>>>>> REPLACE\n"
                   "Rules: SEARCH must match the live file byte-for-byte; "
                   "keep each block minimal (changed lines only); use one "
                   "block per change; repeat blocks for multiple changes.\n\n"
                   "# EXAMPLE (format only — never emit this content)\n"
                   "Given CURRENT STATE containing:\n"
                   "    pub fn progress(&self) -> Option<f64> {\n"
                   "you want to add a method after it. You emit:\n"
                   "```\n"
                   "### src/model.rs\n"
                   "<<<<<<< SEARCH\n"
                   "        Some(self.n_decoded as f64 / total as f64)\n"
                   "    }\n"
                   "}\n"
                   "=======\n"
                   "        Some(self.n_decoded as f64 / total as f64)\n"
                   "    }\n"
                   "\n"
                   "    pub fn total_tokens(&self) -> u64 {\n"
                   "        self.n_decoded + self.n_remain\n"
                   "    }\n"
                   "}\n"
                   ">>>>>>> REPLACE\n"
                   "```\n"
                   "Note how SEARCH is copied verbatim from CURRENT STATE and "
                   "the block includes enough surrounding lines to be unique.")
        else:
            fmt = ("\n\n# OUTPUT FORMAT\n"
                   "Emit each complete file as:\n"
                   "### <relative/path>\n"
                   "<one fenced code block with the FULL file contents>")
        return [{"role": "user", "content": head + fmt}]

    def verifier_messages(self, files: dict[str, str], compile_report: str) -> list[dict]:
        draft_blocks = "\n\n".join(
            f"### {path}\n```{lang_of(path)}\n{content}\n```"
            for path, content in files.items())
        numbered = "\n".join(
            f"I{n+1}: {inv}" for n, inv in enumerate(self.invariants))
        spec = (
            "You are the verifying architect. Audit the DRAFT against EVERY "
            "INVARIANT and the compiler report.\n\n"
            'Reply ONLY with a JSON object:\n'
            '{"pass": <bool>, '
            '"checks": [{"id": <invariant number, e.g. 1 for I1>, '
            '"holds": <bool>, "evidence": "<quote or line ref from the draft>"}], '
            '"delta": ["minimal surgical fix", ...]}\n\n'
            "Rules:\n"
            "- ONE check entry per invariant, identified by its number in id\n"
            "- evidence must cite actual draft content; no evidence means holds=false\n"
            "- delta is empty only when pass=true and every check holds\n\n"
            f"# INVARIANTS\n{numbered}\n\n"
            f"# COMPILER REPORT\n```\n{compile_report or '(clean)'}\n```\n\n"
            f"# DRAFT\n{draft_blocks}"
        )
        return [{"role": "user", "content": spec}]


def lang_of(path: str) -> str:
    return Path(path).suffix.lstrip(".") or "text"


# ── Draft post-processing ────────────────────────────────────────────

THINK_RE = re.compile(r"<think>.*?</think>", re.DOTALL)
THINK_OPEN_RE = re.compile(r"<think>.*$", re.DOTALL)
FENCE_RE = re.compile(r"```[^\n]*\n(.*?)```", re.DOTALL)
SECTION_RE = re.compile(r"^#{3,}[ \t]*(?:FILE:)?[ \t]*`?([^\s`][^\n`]*)`?[ \t]*$",
                        re.MULTILINE)


def strip_thinking(text: str) -> str:
    """Drop closed think blocks; an UNTERMINATED <think> means the budget
    died mid-reasoning — everything after it is dropped too."""
    cleaned = THINK_RE.sub("", text)
    return THINK_OPEN_RE.sub("", cleaned)


def message_text(msg: dict) -> str:
    """Combined visible text of an assistant message (content wins; reasoning
    appended so extractors still find blocks when thinking ate the budget)."""
    parts = [msg.get("content") or "", msg.get("reasoning_content") or ""]
    return "\n".join(p for p in parts if p)


def extract_files(text: str) -> dict[str, str]:
    """Split a multi-file draft into {relative_path: contents}.

    Recognizes `### <path>` (or #### / FILE:) sections, each containing one
    fenced block; the largest fence in the section wins.
    """
    clean = strip_thinking(text)
    matches = list(SECTION_RE.finditer(clean))
    files: dict[str, str] = {}
    if not matches:
        return files
    for i, m in enumerate(matches):
        path = m.group(1).strip()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(clean)
        chunk = clean[m.end():end]
        fences = FENCE_RE.findall(chunk)
        if fences:
            files[path] = max(fences, key=len).strip()
    return files


def extract_code(text: str) -> str:
    """Largest fenced block in the message; falls back to stripped prose."""
    clean = strip_thinking(text)
    blocks = FENCE_RE.findall(clean)
    if blocks:
        return max(blocks, key=len).strip()
    return clean.strip()


@dataclass
class Verdict:
    passed: bool
    checks: list[dict]
    delta: list[str]
    wellformed: bool


def _sanitize_json_candidate(candidate: str) -> str | None:
    """Escape bare control characters inside JSON string literals.

    Model-emitted evidence often quotes multi-line code, producing raw
    newlines inside strings — technically invalid JSON that json.loads
    rejects. Walk the text tracking string state and escape them.
    """
    out = []
    in_str = False
    escaped = False
    for ch in candidate:
        if in_str:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_str = False
            elif ch == "\n":
                out.append("\\n")
                continue
            elif ch == "\t":
                out.append("\\t")
                continue
        elif ch == '"':
            in_str = True
        out.append(ch)
    return "".join(out)


def _find_json_object(text: str) -> dict | None:
    """Extract the most plausible JSON object from mixed prose.

    Scans '{' positions right-to-left with json.raw_decode — reasoning text
    often contains stray braces BEFORE the real verdict, so the last
    well-formed object wins; among ties, one carrying a "pass" key wins.
    Falls back to control-char sanitization for candidates that only fail
    due to raw newlines inside string literals.
    """
    dec = json.JSONDecoder()
    fallback: dict | None = None
    for i in range(len(text) - 1, -1, -1):
        if text[i] != "{":
            continue
        obj = None
        try:
            obj, _end = dec.raw_decode(text, i)
        except json.JSONDecodeError:
            # Find the matching close brace lazily and retry sanitized.
            depth = 0
            j = -1
            for j in range(i, len(text)):
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
                    if depth == 0:
                        break
            if j > i and depth == 0:
                try:
                    obj = json.loads(_sanitize_json_candidate(text[i:j + 1]) or "")
                except json.JSONDecodeError:
                    obj = None
        if isinstance(obj, dict):
            if "pass" in obj:
                return obj
            fallback = fallback or obj
    return fallback


def _norm_inv(s: str) -> str:
    return re.sub(r"[^a-z0-9]+", " ", s.lower()).strip()


def _fuzzy_invariant_match(check_text: str, invariants: list[str]) -> int | None:
    """Index of the invariant best matching a paraphrased check text, or None.

    Hybrid: sequence similarity OR token containment (a terse check like
    'existing methods unchanged' is contained in the longer spec text).
    """
    import difflib
    norm_check = _norm_inv(check_text)
    if not norm_check:
        return None
    check_tokens = set(norm_check.split())
    best, best_score = None, 0.0
    for n, inv in enumerate(invariants):
        norm_inv = _norm_inv(inv)
        seq = difflib.SequenceMatcher(None, norm_check, norm_inv).ratio()
        inv_tokens = set(norm_inv.split())
        contain = (len(check_tokens & inv_tokens) / len(check_tokens)
                   if check_tokens else 0.0)
        score = max(seq, contain)
        if score > best_score:
            best, best_score = n, score
    return best if best_score >= 0.7 else None


def parse_verdict(text: str, invariants: list[str]) -> Verdict:
    """Tolerant verdict parse with invariant-coverage enforcement.

    Checks reference invariants by number (id) per the constrained schema;
    legacy string form falls back to fuzzy matching. A `pass=true` that
    fails to cover every invariant — or shows checks without evidence — is
    coerced to a fail naming the gap.
    """
    clean = strip_thinking(text)
    obj = _find_json_object(clean)
    if isinstance(obj, dict) and isinstance(obj.get("pass"), bool):
        raw_checks = obj.get("checks") or []
        checks = [c for c in raw_checks if isinstance(c, dict)]
        delta = [str(d) for d in (obj.get("delta") or []) if d]
        passed = bool(obj["pass"])

        holds_by_idx: dict[int, bool] = {}
        problems: list[str] = []
        for c in checks:
            idx: int | None = None
            cid = c.get("id")
            if isinstance(cid, int) and 1 <= cid <= len(invariants):
                idx = cid - 1
            else:
                txt = str(c.get("invariant", "") or "")
                idx = _fuzzy_invariant_match(txt, invariants)
            holds = bool(c.get("holds"))
            evidence = str(c.get("evidence", "") or "").strip()
            if not evidence:
                holds = False
            if idx is None:
                continue
            # Any failing evidence for an invariant marks it unheld.
            holds_by_idx[idx] = holds_by_idx.get(idx, True) and holds

        for n, inv in enumerate(invariants):
            if n not in holds_by_idx:
                passed = False
                problems.append(f"unaddressed invariant I{n+1}: {inv}")
            elif not holds_by_idx[n]:
                passed = False
                problems.append(f"failed invariant I{n+1}: {inv}")
        return Verdict(passed, checks, delta + problems, True)

    return Verdict(False, [],
                   [f"UNPARSEABLE VERDICT — reply with strict JSON only. Raw: {clean[:400]}"],
                   False)


# ── HTTP with agentic fault tolerance ────────────────────────────────

VERDICT_SCHEMA = {
    "type": "object",
    "properties": {
        "pass": {"type": "boolean"},
        "checks": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer",
                           "description": "the invariant's I-number"},
                    "holds": {"type": "boolean"},
                    "evidence": {"type": "string"},
                },
                "required": ["id", "holds", "evidence"],
            },
        },
        "delta": {"type": "array", "items": {"type": "string"}},
    },
    "required": ["pass", "checks", "delta"],
}


class ServerDown(Exception):
    """Server unusable for this call. `retryable` = transient (timeout/5xx);
    non-retryable = connection refused (caller may respawn)."""

    def __init__(self, msg: str, retryable: bool = False):
        super().__init__(msg)
        self.retryable = retryable


def _post_chat(url: str, payload: dict, timeout: int) -> tuple[dict, dict]:
    """Single POST; returns (message, usage). Raises ServerDown on failure."""
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{url}/v1/chat/completions", data=body,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        if e.code >= 500:
            raise ServerDown(f"{url} HTTP {e.code}", retryable=True)
        raise
    except urllib.error.URLError as e:
        reason = getattr(e, "reason", None)
        fatal = isinstance(reason, ConnectionRefusedError)
        raise ServerDown(f"{url} unreachable: {reason or e}", retryable=not fatal) from e
    except (TimeoutError, OSError) as e:
        raise ServerDown(f"{url} timed out: {e}", retryable=True) from e
    msg = data["choices"][0]["message"]
    usage = data.get("usage") or {}
    return msg, usage


def chat(url: str, messages: list[dict], max_tokens: int = 4096,
         temperature: float = 0.6, deadline_ts: float | None = None,
         extra_payload: dict | None = None) -> tuple[dict, dict, float]:
    """POST /v1/chat/completions with transient-retry semantics.

    Every attempt recomputes its own timeout from `deadline_ts`, so a call
    can never outlive the wall-clock budget. Connect failures raise a
    non-retryable ServerDown immediately (caller decides on respawn).
    """
    payload = {
        "messages": messages,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "top_p": 0.95,
        "top_k": 20,
        "cache_prompt": True,
    }
    if extra_payload:
        payload.update(extra_payload)
    t0 = time.time()
    last_err: ServerDown | None = None
    for attempt in range(CHAT_RETRIES + 1):
        if deadline_ts is not None:
            remain = int(deadline_ts - time.time())
            if remain < 30:
                raise ServerDown(f"{url}: budget exhausted before attempt",
                                 retryable=False)
            timeout = remain
        else:
            timeout = REQUEST_TIMEOUT
        try:
            msg, usage = _post_chat(url, payload, timeout)
            return msg, usage, time.time() - t0
        except ServerDown as e:
            last_err = e
            if not e.retryable or attempt >= CHAT_RETRIES:
                raise
            time.sleep(2.0)
    raise RuntimeError(f"{url}: retries exhausted: {last_err}")


def health_up(url: str) -> bool:
    try:
        with urllib.request.urlopen(f"{url}/health", timeout=2) as resp:
            return resp.status == 200
    except (urllib.error.URLError, OSError):
        return False


def apply_template(url: str, messages: list[dict]) -> str | None:
    """Render chat messages via the server's jinja template (POST
    /apply-template). None when the endpoint is unavailable."""
    body = json.dumps({"messages": messages}).encode()
    req = urllib.request.Request(
        f"{url}/apply-template", data=body,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read()).get("prompt")
    except (urllib.error.HTTPError, urllib.error.URLError, OSError,
            json.JSONDecodeError, KeyError):
        return None


def completion_constrained(url: str, prompt: str, schema: dict,
                           max_tokens: int = 1024, temperature: float = 0.2,
                           deadline_ts: float | None = None) -> tuple[str, dict, float]:
    """Grammar-constrained generation via POST /completion with top-level
    json_schema — verdicts come out parseable by construction and stop at
    schema completion instead of rambling to the token cap. Raises
    ServerDown on transport failure or endpoint rejection."""
    payload = {
        "prompt": prompt,
        "json_schema": schema,
        "n_predict": max_tokens,
        "temperature": temperature,
        "cache_prompt": True,
    }
    t0 = time.time()
    timeout = REQUEST_TIMEOUT if deadline_ts is None else max(
        30, int(deadline_ts - time.time()))
    body = json.dumps(payload).encode()
    req = urllib.request.Request(
        f"{url}/completion", data=body,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = json.loads(resp.read())
    except urllib.error.HTTPError as e:
        raise ServerDown(f"{url} /completion HTTP {e.code}", retryable=True) from e
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        reason = getattr(e, "reason", None)
        fatal = isinstance(reason, ConnectionRefusedError)
        raise ServerDown(f"{url} /completion failed: {reason or e}",
                         retryable=not fatal) from e
    # Raw /completion shape: content + timings (no OAI choices/usage).
    text = data.get("content")
    if text is None:
        raise ServerDown(f"{url} /completion malformed response", retryable=True)
    t = data.get("timings") or {}
    usage = {
        "prompt_tokens": int(t.get("prompt_n") or 0),
        "completion_tokens": int(t.get("predicted_n") or 0),
    }
    return text, usage, time.time() - t0


def wait_for_server(url: str, timeout: int = SERVER_START_TIMEOUT) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if health_up(url):
            return True
        time.sleep(1.0)
    return False


def ensure_server(url: str, spawn_cmd: str | None, log=print) -> bool:
    """Health-check; on failure optionally spawn and wait for load."""
    if health_up(url):
        return True
    if not spawn_cmd:
        return False
    log(f"[rebis] {url} down — respawning: {spawn_cmd}")
    try:
        subprocess.Popen(shlex.split(spawn_cmd),
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError as e:
        log(f"[rebis] respawn failed: {e}")
        return False
    return wait_for_server(url)


# ── Compiler gate ────────────────────────────────────────────────────

def compiler_gate(cmd: str, workdir: str) -> tuple[bool, str]:
    """Run the objective check (shlex-split, no shell). Returns (passed, out).

    On failure the report leads with a digest of every `error` line so the
    verifier sees the actual blockers first — rustc's tail is mostly summary.
    """
    try:
        proc = subprocess.run(
            shlex.split(cmd), cwd=workdir, capture_output=True,
            text=True, timeout=COMPILE_TIMEOUT,
        )
        out = (proc.stdout + proc.stderr).strip()
        if proc.returncode != 0:
            err_lines = [ln for ln in out.splitlines()
                         if ln.startswith("error") or ln.startswith("warning: unused")]
            digest = "\n".join(err_lines)[:1500]
            return False, (f"ERROR DIGEST:\n{digest}\n\nFULL TAIL:\n{out[-2500:]}")
        return True, out[-4000:]
    except subprocess.TimeoutExpired:
        return False, f"compiler timed out after {COMPILE_TIMEOUT}s"


# ── Journal ──────────────────────────────────────────────────────────

def journal_path(journal_dir: str, task_id: str) -> Path:
    return Path(journal_dir) / f"{task_id}.jsonl"


def journal_append(path: Path, event: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    event = {"ts": datetime.now(timezone.utc).isoformat(timespec="seconds"),
             **event}
    with open(path, "a") as f:
        f.write(json.dumps(event) + "\n")


def load_journal(path: Path) -> list[dict]:
    if not path.exists():
        return []
    events = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return events


def resume_state(events: list[dict]) -> dict:
    """Reconstruct loop position: iterations done, last delta, terminal?"""
    state = {"iterations_done": 0, "delta": None, "terminal": None}
    for ev in events:
        if ev.get("event") == "turn":
            state["iterations_done"] = max(state["iterations_done"],
                                           int(ev.get("iteration", 0)))
            if ev.get("delta"):
                state["delta"] = ev["delta"]
        elif ev.get("event") == "result":
            state["terminal"] = ev
    return state


# ── Accounting ───────────────────────────────────────────────────────

def empty_usage() -> dict:
    return {"prompt_tokens": 0, "completion_tokens": 0}


def add_usage(acc: dict, usage: dict) -> None:
    acc["prompt_tokens"] += int(usage.get("prompt_tokens") or 0)
    acc["completion_tokens"] += int(usage.get("completion_tokens") or 0)


# ── The Rebis loop ───────────────────────────────────────────────────

@dataclass
class TurnRecord:
    iteration: int
    drafter_s: float
    files_written: list[str]
    compile_ok: bool
    verdict_pass: bool
    wellformed: bool
    n_deltas: int
    deltas: list[str]
    drafter_usage: dict
    verifier_usage: dict


def apply_draft(mandatum: Mandatum, files: dict[str, str],
                single: str, log=print) -> tuple[list[str], list[str]]:
    """Write draft files under workdir; returns (written, rejected) paths.

    Safety rails learned from a real incident where a 24-line fragment was
    written over a 230-line source file:
    - FRAGMENT GUARD: an incoming draft under 25% of the current file's size
      (for files >400 chars) is rejected as a suspected truncation.
    - BACKUP: the first overwrite of each target is copied to `<name>.bak`.
    """
    import shutil
    written: list[str] = []
    rejected: list[str] = []
    if files:
        payloads = files
    elif len(mandatum.file_slices) == 1 and single:
        payloads = {mandatum.file_slices[0].path: single}
    else:
        return written, rejected
    for rel, content in payloads.items():
        target = (Path(mandatum.workdir) / rel).resolve()
        if target.exists():
            cur = target.read_text()
            if len(cur) > 400 and len(content) < len(cur) * 0.25:
                rejected.append(rel)
                log(f"[rebis] REJECTED {rel}: draft {len(content)}B is <25% "
                    f"of current {len(cur)}B — suspected truncation")
                continue
            backup = target.with_suffix(target.suffix + ".rebis-bak")
            if not backup.exists():
                shutil.copy2(target, backup)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        written.append(str(target))
    return written, rejected


SR_BLOCK_RE = re.compile(
    r"<<<<<<< SEARCH\s*\n(.*?)\n?={5,9}\s*\n(.*?)\n?>>>>>>>\s*REPLACE",
    re.DOTALL)


def split_sections(text: str) -> dict[str, str]:
    """### <path> sections → {path: section_body} (fence-aware)."""
    clean = strip_thinking(text)
    matches = list(SECTION_RE.finditer(clean))
    out: dict[str, str] = {}
    if not matches:
        return out
    for i, m in enumerate(matches):
        path = m.group(1).strip()
        end = matches[i + 1].start() if i + 1 < len(matches) else len(clean)
        out[path] = clean[m.end():end]
    return out


def apply_replace_blocks(path: Path, body: str, log=print) -> tuple[bool, str]:
    """Apply every SEARCH/REPLACE block in `body` to the file at `path`.

    Exact substring match first; then a whitespace-tolerant line-span match
    (per-line rstrip comparison). Atomic: all blocks must apply or nothing is
    written. Returns (ok, report).
    """
    blocks = SR_BLOCK_RE.findall(body)
    if not blocks:
        return False, "no SEARCH/REPLACE blocks found"
    import shutil
    current = path.read_text() if path.exists() else ""
    backup = path.with_suffix(path.suffix + ".rebis-bak")
    for n, (search, replace) in enumerate(blocks, 1):
        new_current, how = _apply_one_block(current, search, replace)
        if new_current is None:
            return False, (f"block {n}: SEARCH text not found in "
                           f"{path.name} (must be copied verbatim)")
        if how == "fuzzy":
            log(f"[rebis] note: block {n} applied via whitespace-tolerant match")
        current = new_current
    if not backup.exists():
        shutil.copy2(path, backup)
    path.write_text(current)
    return True, f"{len(blocks)} block(s) applied"


def _apply_one_block(current: str, search: str,
                     replace: str) -> tuple[str | None, str]:
    """Apply one block; returns (new_text|None, 'exact'|'fuzzy')."""
    if search and search in current:
        return current.replace(search, replace, 1), "exact"
    # Whitespace-tolerant: match a contiguous span of lines whose rstripped
    # forms equal the search's rstripped lines.
    cur_lines = current.splitlines(keepends=True)
    pat = [ln.rstrip() for ln in search.splitlines()]
    if not pat:
        return None, ""
    n = len(pat)
    for i in range(len(cur_lines) - n + 1):
        if [ln.rstrip() for ln in cur_lines[i:i + n]] == pat:
            new_text = replace + "\n" if not replace.endswith("\n") else replace
            return "".join(cur_lines[:i]) + new_text + "".join(cur_lines[i + n:]), \
                "fuzzy"
    return None, ""


def apply_patch(mandatum: Mandatum, text: str, log=print) -> tuple[bool, str, list[str]]:
    """Apply the drafter's unified diff via `git apply` in workdir.

    Returns (applied, report, touched_files). The raw diff may be fence-wrapped;
    the largest fenced block wins when fences are present.
    """
    candidate = extract_code(text)
    if "diff --git" not in candidate and "---" not in candidate:
        return False, "no unified diff found in draft", []
    if not candidate.endswith("\n"):
        candidate += "\n"
    patch_file = Path(mandatum.workdir) / ".rebis.patch"
    patch_file.write_text(candidate)
    # Tolerance ladder: models miscount hunk headers and hallucinate context
    # lines, so escalate from strict git apply down to fuzzy GNU patch.
    attempts = [
        ["git", "apply", "--whitespace=nowarn", "--recount", "-C1", ".rebis.patch"],
        ["git", "apply", "--whitespace=nowarn", "--recount", ".rebis.patch"],
        ["patch", "-p1", "--fuzz=6", "--batch", "-i", ".rebis.patch"],
    ]
    last_report = ""
    for cmd in attempts:
        try:
            proc = subprocess.run(cmd, cwd=mandatum.workdir,
                                  capture_output=True, text=True, timeout=60)
        except subprocess.TimeoutExpired:
            return False, "git/patch apply timed out", []
        if proc.returncode == 0:
            touched = sorted(set(re.findall(
                r"^\+\+\+ b?/?(?:t/)?(\S+)", candidate, re.MULTILINE)))
            patch_file.unlink(missing_ok=True)
            return True, f"applied via {' '.join(cmd[:2])}", touched
        last_report = (proc.stdout + proc.stderr).strip()
    return False, f"all apply strategies failed: {last_report[-1500:]}", []


def anticipatio_warm(verifier_url: str, mandatum: Mandatum) -> None:
    """Fire-and-forget shadow prefill of the packet's stable prefix.

    Daemon thread; silently ignored on any failure. Post-H1 the gated prompt
    cache turns a warm follow-up turn from ~47s prefill into ~30ms (measured
    2026-08-22, Mellum2 i1-IQ4_XS). Only worth firing when the verifier
    endpoint is not concurrently used by other clients — interleaved
    conversations evict each other's cached states.
    """
    def _warm():
        try:
            msgs = [{"role": "user", "content": mandatum.stable_prefix()}]
            tmpl = apply_template(verifier_url, msgs)
            if not tmpl:
                return
            body = json.dumps({"prompt": tmpl, "n_predict": 1,
                               "temperature": 0.0,
                               "cache_prompt": True}).encode()
            req = urllib.request.Request(
                f"{verifier_url}/completion", data=body,
                headers={"Content-Type": "application/json"}, method="POST")
            urllib.request.urlopen(req, timeout=300).read()
        except Exception:  # noqa: BLE001 - warm is best-effort by design
            pass
    threading.Thread(target=_warm, daemon=True).start()


def rebis_loop(mandatum: Mandatum, drafter_url: str, verifier_url: str,
               journal_dir: str = DEFAULT_JOURNAL_DIR,
               start_iteration: int = 1, start_delta: list[str] | None = None,
               budget_s: float | None = None,
               drafter_spawn: str | None = None,
               verifier_spawn: str | None = None,
               distill_dir: str = DEFAULT_DISTILL_DIR,
               distill: bool = True,
               anticipatio: bool = False,
               log=print) -> dict:
    """Bounded poke-and-refine loop. Returns the run report dict."""
    jpath = journal_path(journal_dir, mandatum.task_id)
    deadline = time.time() + budget_s if budget_s else None
    drafter_acc, verifier_acc = empty_usage(), empty_usage()
    history: list[TurnRecord] = []
    t_start = time.time()
    drec = DistillRecorder(distill_dir, mandatum.task_id, enabled=distill)
    slice_paths = [fs.path for fs in mandatum.file_slices]
    drec.emit("run_open", {
        "objective": mandatum.objective,
        "invariants": mandatum.invariants,
        "constraints": mandatum.constraints,
        "output_contract": mandatum.output_contract,
        "slice_paths": slice_paths,
        "draft_mode": mandatum.draft_mode,
        "verify_mode": mandatum.verify_mode,
        "max_iterations": mandatum.max_iterations,
        "compile_cmd": mandatum.compile_cmd,
    })

    def distill_files(paths) -> dict:
        return drec.snapshot_paths(mandatum.workdir, paths)

    def out_of_budget() -> bool:
        return deadline is not None and time.time() > deadline

    def guarded_chat(url, messages, spawn_cmd, label,
                     max_tokens: int = 4096, temperature: float = 0.6,
                     extra_payload: dict | None = None):
        """chat with respawn-once-on-ServerDown + budget deadline."""
        eff_deadline = (deadline + 30) if deadline else None
        try:
            return chat(url, messages, max_tokens=max_tokens,
                        temperature=temperature, deadline_ts=eff_deadline,
                        extra_payload=extra_payload)
        except ServerDown:
            if not ensure_server(url, spawn_cmd, log):
                raise
            log(f"[rebis] {label} respawned — retrying")
            return chat(url, messages, max_tokens=max_tokens,
                        temperature=temperature, deadline_ts=eff_deadline,
                        extra_payload=extra_payload)

    delta: list[str] | None = start_delta
    files: dict[str, str] = {}
    single = ""
    pause_reason: str | None = None
    iteration = start_iteration

    for iteration in range(start_iteration, mandatum.max_iterations + 1):
        if out_of_budget():
            log("[rebis] wall-clock budget exhausted — pausing (resumable)")
            pause_reason = "budget"
            break

        # 1. Draft (correction turns see the last draft, not the original slice)
        log(f"[rebis] iteration {iteration}/{mandatum.max_iterations}: drafting")
        if mandatum.draft_mode == "patch":
            # Diff hunks need verbatim context: show the LIVE file from disk,
            # not just the packet slice (prefill is cheap; output is what's
            # expensive).
            current_state = {}
            for fs in mandatum.file_slices:
                p = Path(mandatum.workdir) / fs.path
                current_state[fs.path] = p.read_text() if p.exists() else fs.content
            draft_msgs = mandatum.drafter_messages(delta, current_state)
        else:
            draft_msgs = mandatum.drafter_messages(delta, files or None)
        draft_budget = mandatum.draft_budget or \
            (8192 if mandatum.draft_mode == "patch" else 4096)
        # Patch emission is mechanical: thinking burns budget the diff needs.
        patch_extra = {"chat_template_kwargs": {"enable_thinking": False}} \
            if mandatum.draft_mode == "patch" else None
        try:
            raw_msg, d_usage, dt = guarded_chat(drafter_url,
                                                draft_msgs,
                                                drafter_spawn, "drafter",
                                                max_tokens=draft_budget,
                                                extra_payload=patch_extra)
        except ServerDown as e:
            log(f"[rebis] drafter unreachable after respawn: {e}")
            pause_reason = f"drafter down: {e}"
            break
        add_usage(drafter_acc, d_usage)
        if anticipatio:
            anticipatio_warm(verifier_url, mandatum)
        drec.emit("draft", {"iteration": iteration,
                            "text": message_text(raw_msg),
                            "usage": dict(d_usage),
                            "elapsed_s": round(dt, 2)})

        if mandatum.draft_mode == "patch":
            applied, preport, touched = apply_patch(mandatum,
                                                    message_text(raw_msg), log=log)
            if not applied:
                drec.emit("patch_failed", {"iteration": iteration,
                                           "report": preport[-600:]})
                log(f"[rebis] iteration {iteration}: PATCH FAILED")
                history.append(TurnRecord(iteration, round(dt, 2), [],
                                          False, False, True, 1,
                                          [f"your unified diff did not apply: "
                                           f"{preport[:400]} — emit a corrected, "
                                           "complete unified diff against the "
                                           "CURRENT STATE"],
                                          dict(d_usage), {}))
                journal_append(jpath, {"event": "turn", "iteration": iteration,
                                       "patch_failed": True,
                                       "apply_report": preport[-600:]})
                continue
            drec.emit("files_before", {"iteration": iteration,
                                       "files": distill_files(slice_paths)})
            written = touched
            files = {rel: (Path(mandatum.workdir) / rel).read_text()
                     for rel in written
                     if (Path(mandatum.workdir) / rel).exists()}
            drec.emit("files_after", {"iteration": iteration,
                                      "files": distill_files(written)})
            log(f"[rebis] iteration {iteration}: patch applied to {len(written)} file(s)")
        elif mandatum.draft_mode == "replace":
            sections = split_sections(message_text(raw_msg))
            if not sections:
                drec.emit("replace_failed", {"iteration": iteration,
                                             "report": "no ### sections found"})
            if not sections and not extract_code(message_text(raw_msg)):
                log(f"[rebis] iteration {iteration}: EMPTY draft")
                journal_append(jpath, {"event": "turn", "iteration": iteration,
                                       "empty": True})
                continue
            written, failed = [], []
            drec.emit("files_before", {"iteration": iteration,
                                       "files": distill_files(sections.keys())})
            for rel, body in sections.items():
                target = (Path(mandatum.workdir) / rel).resolve()
                if not target.exists():
                    failed.append(f"{rel}: file does not exist")
                    continue
                ok, rep = apply_replace_blocks(target, body, log=log)
                if ok:
                    written.append(str(target))
                else:
                    failed.append(f"{rel}: {rep}")
            if not written:
                msg = "; ".join(failed) or "no ### sections found"
                drec.emit("replace_failed", {"iteration": iteration,
                                             "report": msg[:600]})
                log(f"[rebis] iteration {iteration}: REPLACE FAILED — {msg[:200]}")
                history.append(TurnRecord(iteration, round(dt, 2), [],
                                          False, False, True, 1,
                                          [f"your SEARCH/REPLACE blocks failed to "
                                           f"apply: {msg[:400]} — copy SEARCH "
                                           "lines verbatim from CURRENT STATE"],
                                          dict(d_usage), {}))
                journal_append(jpath, {"event": "turn", "iteration": iteration,
                                       "replace_failed": True,
                                       "apply_report": msg[:600]})
                continue
            files = {rel: (Path(mandatum.workdir) / rel).read_text()
                     for rel in sections
                     if (Path(mandatum.workdir) / rel).exists()}
            drec.emit("files_after", {"iteration": iteration,
                                      "files": distill_files(sections.keys())})
            log(f"[rebis] iteration {iteration}: replace applied to {len(written)} file(s)")
        else:
            files = extract_files(message_text(raw_msg))
            single = extract_code(message_text(raw_msg))
            if not files and not single:
                log(f"[rebis] iteration {iteration}: EMPTY draft "
                    f"(thinking budget exhausted?)")
                journal_append(jpath, {"event": "turn", "iteration": iteration,
                                       "empty": True})
                continue
            drec.emit("files_before", {"iteration": iteration,
                                       "files": distill_files(slice_paths)})
            written, rejected = apply_draft(mandatum, files, single, log=log)
            if rejected:
                drec.emit("fragment_rejected",
                          {"iteration": iteration, "paths": rejected})
            if rejected:
                journal_append(jpath, {"event": "turn", "iteration": iteration,
                                       "rejected_fragments": rejected})
            if not written:
                log(f"[rebis] iteration {iteration}: all payloads rejected "
                    f"(fragment guard)")
                history.append(TurnRecord(iteration, round(dt, 2), [],
                                          False, False, True,
                                          1, ["your previous reply was a truncated "
                                              "fragment; emit the COMPLETE file "
                                              "contents in fenced blocks"],
                                          dict(d_usage), {}))
                continue
            if written:
                drec.emit("files_after", {"iteration": iteration,
                                          "files": distill_files(slice_paths)})
            log(f"[rebis] iteration {iteration}: wrote {len(written)} file(s)")

        # 2. Compiler gate
        if mandatum.compile_cmd:
            compile_ok, report = compiler_gate(mandatum.compile_cmd, mandatum.workdir)
        else:
            compile_ok, report = True, ""
        drec.emit("gate", {"iteration": iteration, "compile_ok": compile_ok,
                           "report": report[-600:]})

        # 3. Verifier audits every draft — compilers cannot see semantic
        #    invariants ("the buffer must be freed", not merely nulled).
        status = "GREEN" if compile_ok else "RED"
        if mandatum.verify_mode == "compiler_only" and compile_ok:
            # Every invariant is enforced by the gate (tests/greps) — the LLM
            # auditor would only add hallucination surface (observed: it
            # ordered fixes for code that already existed).
            log(f"[rebis] iteration {iteration}: ACCEPTED "
                f"(compiler_only: gate green)")
            history.append(TurnRecord(iteration, round(dt, 2), written,
                                      True, True, True, 0, [],
                                      dict(d_usage), {}))
            wall = round(time.time() - t_start, 2)
            drec.emit("run_close", {"accepted": True, "wall_s": wall,
                                    "totals": {"drafter": drafter_acc,
                                               "verifier": verifier_acc}})
            report = build_report(mandatum.task_id, True, history,
                                  drafter_acc, verifier_acc, wall)
            journal_append(jpath, {"event": "result", "accepted": True,
                                   "wall_s": wall})
            return report
        log(f"[rebis] iteration {iteration}: compiler {status} — verifier reviewing")
        v_msgs = mandatum.verifier_messages(
            files or {mandatum.file_slices[0].path: single}, report)

        def call_with_respawn(fn, url, spawn_cmd, label):
            try:
                return fn()
            except ServerDown:
                if not ensure_server(url, spawn_cmd, log):
                    raise
                log(f"[rebis] {label} respawned — retrying")
                return fn()

        def call_constrained():
            # Grammar-constrained verdict (/completion + json_schema):
            # parseable by construction, no rambling to the token cap.
            tmpl = apply_template(verifier_url, v_msgs)
            if not tmpl:
                raise ServerDown("apply-template unavailable", retryable=False)
            return completion_constrained(
                verifier_url, tmpl, VERDICT_SCHEMA, max_tokens=1024,
                temperature=0.0,
                deadline_ts=(deadline + 30) if deadline else None)

        def call_legacy():
            msg, usage, _t = chat(
                verifier_url, v_msgs, max_tokens=2048, temperature=0.2,
                deadline_ts=(deadline + 30) if deadline else None)
            return message_text(msg), usage, 0.0

        try:
            vtext, v_usage, _vt = call_with_respawn(
                call_constrained, verifier_url, verifier_spawn, "verifier")
            verdict = parse_verdict(vtext, mandatum.invariants)
        except ServerDown as e:
            log(f"[rebis] constrained verdict unavailable ({e}) — legacy chat")
            try:
                vtext, v_usage, _vt = call_with_respawn(
                    call_legacy, verifier_url, verifier_spawn, "verifier")
            except ServerDown as e2:
                log(f"[rebis] verifier unreachable after respawn: {e2}")
                pause_reason = f"verifier down: {e2}"
                break
            verdict = parse_verdict(vtext, mandatum.invariants)
        add_usage(verifier_acc, v_usage)
        drec.emit("verdict", {"iteration": iteration,
                              "pass": verdict.passed,
                              "wellformed": verdict.wellformed,
                              "delta": verdict.delta})

        rec = TurnRecord(iteration, round(dt, 2), written, compile_ok,
                         verdict.passed, verdict.wellformed,
                         len(verdict.delta), verdict.delta,
                         dict(d_usage), dict(v_usage))
        history.append(rec)
        journal_append(jpath, {"event": "turn", "iteration": iteration,
                               "compile_ok": compile_ok,
                               "verdict_pass": verdict.passed,
                               "wellformed": verdict.wellformed,
                               "delta": verdict.delta,
                               "files": written})

        if compile_ok and verdict.passed:
            wall = round(time.time() - t_start, 2)
            drec.emit("run_close", {"accepted": True, "wall_s": wall,
                                    "totals": {"drafter": drafter_acc,
                                               "verifier": verifier_acc}})
            log(f"[rebis] iteration {iteration}: ACCEPTED "
                f"(compile green, verifier pass)")
            report = build_report(mandatum.task_id, True, history,
                                  drafter_acc, verifier_acc, wall)
            journal_append(jpath, {"event": "result", "accepted": True,
                                   "wall_s": wall})
            return report

        delta = verdict.delta or [
            "previous draft rejected; produce corrected COMPLETE files"]
        log(f"[rebis] iteration {iteration}: poke — {len(delta)} correction order(s)")

    wall = round(time.time() - t_start, 2)
    drec.emit("run_close", {"accepted": False, "paused": pause_reason is not None,
                            "pause_reason": pause_reason, "wall_s": wall,
                            "totals": {"drafter": drafter_acc,
                                       "verifier": verifier_acc}})
    report = build_report(mandatum.task_id, False, history,
                          drafter_acc, verifier_acc, wall)
    if pause_reason:
        # Resumable abort — NOT terminal; --resume continues from here.
        journal_append(jpath, {"event": "paused", "reason": pause_reason,
                               "next_iteration": iteration, "wall_s": wall})
    else:
        journal_append(jpath, {"event": "result", "accepted": False,
                               "reason": "iteration cap", "wall_s": wall})
    return report


def build_report(task_id: str, accepted: bool, history: list[TurnRecord],
                 drafter_acc: dict, verifier_acc: dict, wall_s: float) -> dict:
    return {
        "task_id": task_id,
        "accepted": accepted,
        "iterations_used": len(history),
        "turns": [asdict(t) for t in history],
        "totals": {
            "drafter": drafter_acc,
            "verifier": verifier_acc,
            "wall_s": wall_s,
        },
    }


def baseline_run(mandatum: Mandatum, verifier_url: str,
                 distill_dir: str = DEFAULT_DISTILL_DIR,
                 distill: bool = True, log=print) -> dict:
    """A/B control: whole task straight to the big model, no loop."""
    drec = DistillRecorder(distill_dir, f"{mandatum.task_id}-baseline",
                           enabled=distill)
    slice_paths = [fs.path for fs in mandatum.file_slices]
    drec.emit("run_open", {"objective": mandatum.objective,
                           "invariants": mandatum.invariants,
                           "constraints": mandatum.constraints,
                           "slice_paths": slice_paths,
                           "mode": "baseline"})
    acc = empty_usage()
    msgs = [{"role": "user", "content":
             f"{mandatum.stable_prefix()}\n\n{mandatum.volatile_body()}\n\n"
             "Write the complete solution.\n\n# OUTPUT FORMAT\n"
             "Emit each complete file as:\n### <relative/path>\n"
             "<one fenced code block with the FULL file contents>"}]
    t0 = time.time()
    raw_msg, usage, _dt = chat(verifier_url, msgs, max_tokens=4096)
    add_usage(acc, usage)
    drec.emit("draft", {"text": message_text(raw_msg), "usage": dict(usage)})
    drec.emit("files_before", {"files": drec.snapshot_paths(mandatum.workdir,
                                                            slice_paths)})
    files = extract_files(message_text(raw_msg))
    single = extract_code(message_text(raw_msg))
    written, _rejected = apply_draft(mandatum, files, single, log=log)
    drec.emit("files_after", {"files": drec.snapshot_paths(mandatum.workdir,
                                                           slice_paths)})
    if mandatum.compile_cmd:
        ok, gate_report = compiler_gate(mandatum.compile_cmd, mandatum.workdir)
    else:
        ok, gate_report = True, ""
    drec.emit("gate", {"compile_ok": ok, "report": gate_report[-600:]})
    wall = round(time.time() - t0, 2)
    drec.emit("run_close", {"accepted": bool(ok), "wall_s": wall,
                            "totals": {"drafter": acc}})
    log(f"[baseline] single-shot {'GREEN' if ok else 'RED'} ({wall}s, "
        f"{len(written)} file(s))")
    return build_report(f"{mandatum.task_id}-baseline", ok, [], acc,
                        empty_usage(), wall)


# ── Self-test (pure functions) ───────────────────────────────────────

def selftest() -> int:
    fs = FileSlice("src/a.rs", 1, 5, "fn old() {}")
    m = Mandatum(objective="obj", invariants=["inv one", "inv two"],
                 constraints=["c"], output_contract="fenced",
                 file_slices=[fs])
    msgs = m.drafter_messages()
    body = msgs[0]["content"]
    assert body.index("# INVARIANTS") < body.index("# FILE"), \
        "stable prefix must precede volatile slices"
    assert "### <relative/path>" in body

    dm = m.drafter_messages(["zero the pointer"])
    assert "CORRECTION ORDERS" in dm[0]["content"]

    # Multi-file extraction
    draft = (
        "<think>reasoning here</think>\n"
        "### src/a.rs\n```rust\nfn a() {}\n```\n"
        "#### src/deep/b.rs\n```rust\nfn b() {}\n```\n")
    files = extract_files(draft)
    assert files == {"src/a.rs": "fn a() {}", "src/deep/b.rs": "fn b() {}"}, files
    assert extract_files("no sections, ```\nlone fence\n```") == {}

    # Verdict: fully evidenced pass accepted
    good = ('{"pass": true, "checks": ['
            '{"invariant": "inv one", "holds": true, "evidence": "line 4"},'
            '{"invariant": "inv two", "holds": true, "evidence": "line 9"}], '
            '"delta": []}')
    v = parse_verdict(good, ["inv one", "inv two"])
    assert v.passed and v.wellformed and not v.delta

    # Missing invariant coverage coerces fail even with pass=true
    lazy = '{"pass": true, "checks": [{"invariant": "inv one", "holds": true, "evidence": "l4"}], "delta": []}'
    v2 = parse_verdict(lazy, ["inv one", "inv two"])
    assert not v2.passed and any("unaddressed" in d for d in v2.delta)

    # Evidence-free hold counts as failed
    no_ev = '{"pass": true, "checks": [{"invariant": "inv one", "holds": true, "evidence": ""},{"invariant": "inv two", "holds": true, "evidence": "x"}], "delta": []}'
    v3 = parse_verdict(no_ev, ["inv one", "inv two"])
    assert not v3.passed

    # Failed hold propagates
    bad = '{"pass": false, "checks": [{"invariant": "inv one", "holds": false, "evidence": "missing free"}], "delta": ["free the buffer"]}'
    v4 = parse_verdict(bad, ["inv one", "inv two"])
    assert not v4.passed and "free the buffer" in v4.delta

    # Garbage stays unparseable
    v5 = parse_verdict("<think>x</think>no json at all", ["inv one"])
    assert not v5.passed and not v5.wellformed and "UNPARSEABLE" in v5.delta[0]

    # Raw newlines inside evidence strings are repaired, not fatal
    broken = ('{"pass": true, "checks": ['
              '{"invariant": "inv one", "holds": true, "evidence": "line1\nline2"},'
              '{"invariant": "inv two", "holds": true, "evidence": "ok"}], '
              '"delta": []}')
    v6 = parse_verdict(broken, ["inv one", "inv two"])
    assert v6.passed and v6.wellformed

    # Id-based checks (constrained schema) resolve by number
    ids = ('{"pass": true, "checks": ['
           '{"id": 2, "holds": true, "evidence": "l9"},'
           '{"id": 1, "holds": true, "evidence": "l4"}], "delta": []}')
    v7 = parse_verdict(ids, ["inv one", "inv two"])
    assert v7.passed and v7.wellformed

    # Paraphrased legacy strings fall back to fuzzy matching (realistic case:
    # terse check contained in long spec text — the f1-v4 failure mode)
    para = ('{"pass": true, "checks": ['
            '{"invariant": "inv one", "holds": true, "evidence": "x"},'
            '{"invariant": "existing commit reset methods unchanged", '
            '"holds": true, "evidence": "y"}], "delta": []}')
    v8 = parse_verdict(
        para,
        ["inv one",
         "existing commit/reset methods keep their exact behavior; "
         "you MAY add derives or constructors when the test needs them"])
    assert v8.passed and v8.wellformed

    # Out-of-range ids fall back to nothing → uncovered
    bad_id = '{"pass": true, "checks": [{"id": 9, "holds": true, "evidence": "?"}], "delta": []}'
    v9 = parse_verdict(bad_id, ["inv one"])
    assert not v9.passed and any("unaddressed" in d for d in v9.delta)

    # Journal roundtrip + resume reconstruction
    import tempfile, os
    with tempfile.TemporaryDirectory() as td:
        jp = Path(td) / "t.jsonl"
        journal_append(jp, {"event": "turn", "iteration": 1, "delta": ["d1"]})
        journal_append(jp, {"event": "turn", "iteration": 2, "delta": ["d2"]})
        st = resume_state(load_journal(jp))
        assert st["iterations_done"] == 2 and st["delta"] == ["d2"]
        assert st["terminal"] is None
        journal_append(jp, {"event": "result", "accepted": True})
        st2 = resume_state(load_journal(jp))
        assert st2["terminal"] and st2["terminal"]["accepted"]

    # Usage accumulation tolerates junk
    acc = empty_usage()
    add_usage(acc, {"prompt_tokens": 10, "completion_tokens": 5})
    add_usage(acc, {})
    assert acc == {"prompt_tokens": 10, "completion_tokens": 5}

    # Compiler gate
    ok, _ = compiler_gate("true", ".")
    assert ok
    ok2, _ = compiler_gate("false", ".")
    assert not ok2

    # Fragment guard: tiny draft over a big existing file is rejected, and a
    # .rebis-bak snapshot is left on first legitimate overwrite.
    with tempfile.TemporaryDirectory() as td:
        big = Path(td) / "big.rs"
        big.write_text("// " + "x" * 400 + "\nfn main() {}\n")
        m2 = Mandatum(objective="o", workdir=td,
                      file_slices=[FileSlice("big.rs", 1, 2, "old")])
        written, rejected = apply_draft(m2, {"big.rs": "fn f() {}"}, "", log=lambda *_: None)
        assert not written and rejected == ["big.rs"]
        assert len(big.read_text()) > 400  # untouched
        good = "// rewritten legitimately\n" + "fn a() {}\n" * 20
        written, _ = apply_draft(m2, {"big.rs": good}, "", log=lambda *_: None)
        assert written and Path(str(big) + ".rebis-bak").exists()

    # Patch mode: valid diff applies, invalid diff reports failure.
    import subprocess as sp
    with tempfile.TemporaryDirectory() as td:
        sp.run(["git", "init", "-q"], cwd=td)
        Path(td, "t.txt").write_text("alpha\nbeta\n")
        sp.run(["git", "add", "."], cwd=td)
        sp.run(["git", "-c", "user.email=r@x", "-c", "user.name=r", "commit", "-qm", "i"], cwd=td)
        m3 = Mandatum(objective="o", workdir=td)
        good_diff = (
            "diff --git a/t.txt b/t.txt\n"
            "--- a/t.txt\n"
            "+++ b/t.txt\n"
            "@@ -1,2 +1,3 @@\n"
            " alpha\n"
            "+gamma\n"
            " beta\n")
        ok, rep, touched = apply_patch(m3, f"```diff\n{good_diff}```")
        assert ok and touched == ["t.txt"], (ok, rep, touched)
        assert "gamma" in Path(td, "t.txt").read_text()
        bad = good_diff.replace("alpha\n+gamma", "alpha\n+gamma\n+zeta")
        ok2, rep2, _ = apply_patch(m3, bad)
        assert not ok2 and "apply strategies failed" in rep2

    # Header-less diff (model drops the diff --git line) still applies.
    with tempfile.TemporaryDirectory() as td:
        sp.run(["git", "init", "-q"], cwd=td)
        Path(td, "t.txt").write_text("alpha\nbeta\n")
        sp.run(["git", "add", "."], cwd=td)
        sp.run(["git", "-c", "user.email=r@x", "-c", "user.name=r", "commit", "-qm", "i"], cwd=td)
        m4 = Mandatum(objective="o", workdir=td)
        bare = "--- a/t.txt\n+++ b/t.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+beta v2\n"
        ok3, rep3, touched3 = apply_patch(m4, bare)
        assert ok3 and touched3 == ["t.txt"], rep3
        assert "beta v2" in Path(td, "t.txt").read_text()

    # SEARCH/REPLACE blocks: valid applies, missing-anchor fails atomically.
    with tempfile.TemporaryDirectory() as td:
        target = Path(td) / "t.rs"
        target.write_text("fn a() {}\nfn b() {}\n")
        m5 = Mandatum(objective="o", workdir=td)
        body = ("### t.rs\n"
                "<<<<<<< SEARCH\n"
                "fn b() {}\n"
                "=======\n"
                "fn b() { /* v2 */ }\n"
                ">>>>>>> REPLACE\n")
        ok, rep = apply_replace_blocks(target, body)
        assert ok and "v2" in target.read_text(), rep
        bad_body = body.replace("fn b() {}", "fn nonexistent() {}")
        ok2, rep2 = apply_replace_blocks(target, bad_body)
        assert not ok2 and "not found" in rep2
        assert "v2" in target.read_text()  # atomic: failed run left file intact

    print("selftest: all assertions passed")
    return 0


# ── CLI ──────────────────────────────────────────────────────────────

def main() -> int:
    p = argparse.ArgumentParser(description="REBIS drafter/verifier loop")
    p.add_argument("--task", help="Mandatum task JSON")
    p.add_argument("--drafter-url", default="http://127.0.0.1:8287")
    p.add_argument("--verifier-url", default="http://127.0.0.1:8279")
    p.add_argument("--mode", choices=["rebis", "baseline"], default="rebis")
    p.add_argument("--selftest", action="store_true",
                   help="run pure-function assertions")
    p.add_argument("--resume", metavar="TASK_ID",
                   help="resume a journaled task instead of starting fresh")
    p.add_argument("--task-id", default=None,
                   help="override task id (journal name)")
    p.add_argument("--journal-dir", default=DEFAULT_JOURNAL_DIR)
    p.add_argument("--budget-s", type=float, default=None,
                   help="wall-clock budget in seconds")
    p.add_argument("--report", default=None, help="write final report JSON here")
    p.add_argument("--drafter-spawn", default=None,
                   help="command to spawn drafter server when down")
    p.add_argument("--verifier-spawn", default=None,
                   help="command to spawn verifier server when down")
    p.add_argument("--distill-dir", default=DEFAULT_DISTILL_DIR,
                   help=f"training-data capture directory (default: {DEFAULT_DISTILL_DIR})")
    p.add_argument("--no-distill", action="store_true",
                   help="disable training-data capture")
    p.add_argument("--anticipatio", action="store_true",
                   help="shadow-prefill the verifier's stable prefix after "
                        "each Mandatum (single-client endpoints only)")
    args = p.parse_args()

    if args.selftest:
        return selftest()
    if not args.task:
        p.error("--task is required unless --selftest")

    mandatum = Mandatum.load(args.task)
    if args.task_id:
        mandatum.task_id = args.task_id

    start_iter, start_delta = 1, None
    if args.resume:
        events = load_journal(journal_path(args.journal_dir, args.resume))
        st = resume_state(events)
        if st["terminal"]:
            print(f"[rebis] task {args.resume} already finished: "
                  f"accepted={st['terminal'].get('accepted')}")
            return 0 if st["terminal"].get("accepted") else 1
        start_iter = st["iterations_done"] + 1
        start_delta = st["delta"]
        mandatum.task_id = args.resume
        print(f"[rebis] resuming {args.resume} at iteration {start_iter}")

    for name, url in [("drafter", args.drafter_url),
                      ("verifier", args.verifier_url)]:
        if not ensure_server(url, None):
            print(f"[rebis] WARNING: {name} not answering at {url}")

    if args.mode == "baseline":
        report = baseline_run(mandatum, args.verifier_url,
                              distill_dir=args.distill_dir,
                              distill=not args.no_distill)
    else:
        report = rebis_loop(
            mandatum, args.drafter_url, args.verifier_url,
            journal_dir=args.journal_dir,
            start_iteration=start_iter, start_delta=start_delta,
            budget_s=args.budget_s,
            drafter_spawn=args.drafter_spawn,
            verifier_spawn=args.verifier_spawn,
            distill_dir=args.distill_dir,
            distill=not args.no_distill,
            anticipatio=args.anticipatio)

    print(json.dumps({"task_id": report["task_id"],
                      "accepted": report["accepted"],
                      "iterations": report["iterations_used"],
                      "totals": report["totals"]}, indent=2))
    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2))
    return 0 if report["accepted"] else 1


if __name__ == "__main__":
    sys.exit(main())
