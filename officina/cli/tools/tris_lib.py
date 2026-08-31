"""tris_lib — shared data-plane helpers for the tris CLI (Round 4, T1).

Schemas are defined in docs/TRIS-EXPERIENCE.md (single source). Everything
here is best-effort by contract: an event or ledger append must NEVER break
the feature that emits it (Rule 15 discipline extended to observability).
"""

from __future__ import annotations

import json
import os
import time
import urllib.request
from pathlib import Path
from typing import Any

# SS4 (2026-08-31): state consolidated under ~/.vitriol/officina/state;
# a legacy ~/.local/state/trismegistus store is migrated (moved) on first
# use so ledgers and events survive the fold-in.
_DEFAULT_STATE = Path.home() / ".vitriol" / "officina" / "state"
_LEGACY_STATE = Path.home() / ".local/state/trismegistus"


def _migrate_state() -> Path:
    if _LEGACY_STATE.exists() and not _DEFAULT_STATE.exists():
        try:
            _DEFAULT_STATE.parent.mkdir(parents=True, exist_ok=True)
            _LEGACY_STATE.rename(_DEFAULT_STATE)
        except OSError:
            return _LEGACY_STATE  # move failed; keep serving the old store
    return _DEFAULT_STATE


STATE_DIR = Path(os.environ.get("TRIS_STATE_DIR") or _migrate_state())


def events_path() -> Path:
    """Resolved at CALL time (tests / TRIS_STATE_DIR overrides redirect it)."""
    return STATE_DIR / "events.jsonl"


def ledger_path() -> Path:
    return STATE_DIR / "ledger.jsonl"

EVENT_SOURCES = {"lc-clearer", "lc-rtk", "lc-ckpt", "lc-relay", "lc-tasks", "lc-perms", "vb-gate", "vb-dispatch", "lc-lane", "tris"}


def emit_event(src: str, ev: str, detail: str = "", **fields: Any) -> None:
    """Append one pipeline-stage event; swallows every error by design."""
    if src not in EVENT_SOURCES:
        return
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        rec = {"ts": time.time(), "src": src, "ev": ev, "detail": detail[:200], **fields}
        with events_path().open("a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
    except OSError:
        pass


def tail_events(n: int = 30) -> list[dict[str, Any]]:
    """Last n event lines (newest last); [] when the store is absent/rotten."""
    try:
        lines = events_path().read_text(encoding="utf-8", errors="replace").splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in lines:
        try:
            out.append(json.loads(line))
        except ValueError:
            continue
    return out


def metrics_snapshot(endpoint: str) -> dict[str, float] | None:
    """Parse Prometheus text (llamacpp:*_total counters) -> ledger vocabulary.
    Fork names verified live 2026-08-29: prompt_tokens_total,
    generated_tokens_total, read_bytes_total (KV cache reads), write_bytes_total."""
    try:
        with urllib.request.urlopen(endpoint.rstrip("/") + "/metrics", timeout=4) as res:
            text = res.read().decode("utf-8", "replace")
    except OSError:
        return None
    # Counter names verified against THIS fork build live 2026-08-29 (upstream
    # generated_tokens_total does not exist here; n_decode_total is decode
    # iterations ~= generated tokens at parallel=1, spec-decode off).
    # No KV-read byte counter in this build -> cached_bytes stays None (the
    # engine telemetry TODO covers both n_kv and cache bytes).
    wanted = {
        "llamacpp:prompt_tokens_total": "prompt_tokens",
        "llamacpp:n_decode_total": "generated_tokens",
    }
    out: dict[str, float] = {}
    for line in text.splitlines():
        key, sep, val = line.partition(" ")
        if not sep or key not in wanted:
            continue
        try:
            out[wanted[key]] = float(val)
        except ValueError:
            continue
    return out or None


def diff_metrics(before: dict[str, float] | None, after: dict[str, float] | None) -> dict[str, float]:
    """Counter deltas (never guesses): missing side -> {}."""
    if not before or not after:
        return {}
    return {k: max(0.0, after[k] - before.get(k, 0.0)) for k in after if k in before}


def append_ledger(record: dict[str, Any]) -> None:
    """One task record; swallows OSError (the ledger must not eat a run)."""
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        with ledger_path().open("a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
    except OSError:
        pass


def read_ledger(n: int = 200) -> list[dict[str, Any]]:
    try:
        lines = ledger_path().read_text(encoding="utf-8", errors="replace").splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in lines:
        try:
            out.append(json.loads(line))
        except ValueError:
            continue
    return out


# ── GPU lane arbiter (mining plan M8 MVP — READ-ONLY, 2026-08-31) ─────────

DEFAULT_LANES = {
    "master": "http://127.0.0.1:8279",
    "crush-small": "http://127.0.0.1:8287",
}


def _get_json(url: str, timeout_s: float) -> dict[str, Any] | None:
    try:
        with urllib.request.urlopen(url, timeout=timeout_s) as res:
            return json.loads(res.read().decode("utf-8", "replace"))
    except (OSError, ValueError):
        return None


def lane_snapshot(lanes: dict[str, str] | None = None, timeout_s: float = 2.0) -> dict[str, Any]:
    """Read-only state of every configured GPU lane.

    For each lane: /health + /props truth (model id, n_ctx, vision) —
    Rule 5: state is VERIFIED against the engine, never guessed from
    profiles. Also the active fingerprint and nvidia-smi memory lines
    when the tools are present. Never raises (observability contract).
    """
    out: dict[str, Any] = {"lanes": {}, "fingerprint": fingerprint_from_log(), "gpus": []}
    for name, base in (lanes or DEFAULT_LANES).items():
        base = base.rstrip("/")
        health = _get_json(base + "/health", timeout_s)
        if not health:
            out["lanes"][name] = {"endpoint": base, "status": "down"}
            continue
        props = _get_json(base + "/props", timeout_s) or {}
        gen = props.get("default_generation_settings", {}) if isinstance(props, dict) else {}
        modal = props.get("modalities", {}) if isinstance(props, dict) else {}
        out["lanes"][name] = {
            "endpoint": base,
            "status": "ok",
            "model": gen.get("model") or props.get("model_path"),
            "n_ctx": gen.get("n_ctx"),
            "vision": bool(modal.get("vision", False)),
        }
    try:
        import subprocess

        res = subprocess.run(
            ["nvidia-smi", "--query-gpu=index,name,memory.used,memory.total",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=5)
        if res.returncode == 0:
            out["gpus"] = [l.strip() for l in res.stdout.splitlines() if l.strip()]
    except (OSError, subprocess.TimeoutExpired):
        pass
    return out



def fingerprint_from_log(log_path: str = "/tmp/opencode/vitriol-serve.log",
                         unit_cmd: str = "journalctl --user -u vitriol-server.service --no-pager -n 400") -> str:
    """Newest VITRIOL-FINGERPRINT. Journal FIRST when the unit is active —
    the nohup log goes stale once the supervisor owns the server (2026-08-29:
    a dead log line got recorded next to a unit-run engine)."""
    journal = _fingerprint_from(_journal(unit_cmd))
    if journal:
        return journal
    try:
        return _fingerprint_from(Path(log_path).read_text(errors="replace"))
    except OSError:
        return ""


def _journal(unit_cmd: str) -> str:
    import subprocess

    try:
        active = subprocess.run(["systemctl", "--user", "is-active", "vitriol-server.service"],
                                capture_output=True, text=True, timeout=5).stdout.strip()
        if active != "active":
            return ""
        return subprocess.run(unit_cmd.split(), capture_output=True, text=True, timeout=8).stdout
    except (OSError, subprocess.TimeoutExpired):
        return ""


def _fingerprint_from(text: str) -> str:
    hits = [l[l.index("VITRIOL-FINGERPRINT"):] for l in text.splitlines() if "VITRIOL-FINGERPRINT" in l]
    return hits[-1][:200] if hits else ""


def model_certified(fingerprint: str) -> bool:
    """DEV/CERT badge: CERTIFIED only on the dual-GPU 27B master config
    (tensor_split present); a 9B single-GPU run is always DEV (Rule 4)."""
    return "27B" in fingerprint and "ts=" in fingerprint and "ts=none" not in fingerprint


def budget_table(cfg_text: str) -> list[dict[str, Any]]:
    """§R2.8 allocation rows extracted from the unified config (budget ints).

    Iterative DFS (praetor flags tree recursion; the explicit stack also
    keeps depth visible for the nesting rule)."""
    import yaml

    alloc: list[dict[str, Any]] = []
    try:
        cfg = yaml.safe_load(cfg_text)
    except Exception:  # noqa: BLE001 — render what's parseable, never crash the panel
        return alloc
    stack: list[tuple[Any, str]] = [(cfg, "")]
    while stack:
        node, path = stack.pop()
        if not isinstance(node, dict):
            continue
        budget = node.get("budget")
        if path and isinstance(budget, (int, float)):
            alloc.append({"item": path, "budget": int(budget)})
        stack.extend((v, f"{path}.{k}" if path else str(k)) for k, v in node.items() if isinstance(v, dict))
    return alloc


# ── permissions snapshot (policy mirror for the scaffold runtime) ─────────

def perms_snapshot(cfg: dict) -> dict:
    """Canonical snapshot of safety.permissions (+default_action).

    The node runtime has no YAML parser, so the config's policy reaches
    little-coder through this JSON mirror + source_hash; trismegistus
    validate fails when the mirror drifts from the yaml (drift is a bug).
    """
    import hashlib

    safety = cfg.get("safety", {}) if isinstance(cfg.get("safety"), dict) else {}
    default = str(safety.get("default_action", "allow"))
    if default not in ("allow", "deny", "ask"):
        default = "allow"
    rules = []
    for r in safety.get("permissions", []) or []:
        if not isinstance(r, dict):
            continue
        rules.append({
            "tool": str(r.get("tool", "")).lower(),
            "pattern": str(r.get("path", "")),
            "action": str(r.get("action", "allow")).lower(),
        })
    payload = json.dumps({"default_action": default, "rules": rules}, sort_keys=True)
    return {"default_action": default, "rules": rules, "source_hash": hashlib.sha256(payload.encode()).hexdigest()[:16]}


def perms_snapshot_path() -> Path:
    return Path(os.environ.get("TRIS_PERMS_FILE", str(Path.home() / ".config/trismegistus/permissions.json")))


def sync_perms(cfg: dict) -> Path:
    """Write the mirror atomically-ish (tmp + replace). Returns path."""
    path = perms_snapshot_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(perms_snapshot(cfg), indent=1) + "\n")
    tmp.replace(path)
    return path
