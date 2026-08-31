"""Trismegistus unified-config validator (REPORT-02 step 23, scaffold half).

Layer Interface Protocol: "~/.config/trismegistus/config.yaml generates or
validates per-component configs. Duplicate config files drift; drift is a
bug." This CLI is the VALIDATE half (generate/sync lands with the cert
harness, post-reboot R3 territory).

Checks (each: PASS / WARN / FAIL with file: why):
  structure    config parses, required keys present
  rule2-shim   engine shim must be flag-off (Trismegistus contract)
  rule6-cert   cert_required must be true
  rule15-kill  every context stage has an `enabled` kill switch
  pipeline     order == [clear, compact, compress] (Rule 8)
  profiles     referenced VITRIOL profiles exist + model paths exist
  parity-port  port identical across unified config / profiles /
               little-coder models.json / hermes custom_providers
               (stale entries are bugs per AGENTS.md)
The cert-suite execution gate itself is VITRIOL-side (post-reboot).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import yaml

HOME = Path.home()

REQUIRED_PATHS = [
    "engine.vitriol.endpoint",
    "coding.context_pipeline.order",
    "safety.approval_required",
]

KILL_SWITCH_PATHS = [
    "coding.context_pipeline.clear.tool_result_clearer",
    "coding.context_pipeline.clear.rtk_output",
    "coding.context_pipeline.compact.async_compaction",
    "coding.context_pipeline.compact.batch_aware",
    "coding.context_pipeline.compress.llmlingua2",
    "coding.context_pipeline.compress.caveman_rules",
    "coding.injection.repo_map",
    "coding.injection.task_state",
    "coding.injection.diagnostics_loop",
    "coding.injection.snapshot",
    "coding.injection.vitriol_checkpoint",
    "coding.injection.hermes_bridge",
    "coding.injection.rewoo_dispatch",
    "gateway.hermes.memory_extractor",
    "gateway.hermes.context_relay",
    "gateway.hermes.injection_guard",
]


@dataclass
class Result:
    name: str
    ok: bool
    soft: bool  # WARN vs FAIL
    detail: str


@dataclass
class Report:
    results: list[Result] = field(default_factory=list)

    @property
    def failed(self) -> list[Result]:
        return [r for r in self.results if not r.ok and not r.soft]

    @property
    def warned(self) -> list[Result]:
        return [r for r in self.results if not r.ok and r.soft]

    @property
    def exit_code(self) -> int:
        return 1 if self.failed else 0


def dig(cfg: dict, dotted: str) -> Any:
    cur: Any = cfg
    for part in dotted.split("."):
        if not isinstance(cur, dict) or part not in cur:
            return None
        cur = cur[part]
    return cur


def load_unified(path: Path) -> dict:
    with open(path, encoding="utf-8") as f:
        data = yaml.safe_load(f)
    if not isinstance(data, dict):
        raise ValueError("unified config is not a mapping")
    return data


def check_structure(cfg: dict) -> Result:
    missing = [p for p in REQUIRED_PATHS if dig(cfg, p) is None]
    if missing:
        return Result("structure", False, False, "missing keys: " + ", ".join(missing))
    return Result("structure", True, False, "required keys present")


def check_rule2_shim(cfg: dict) -> Result:
    enabled = dig(cfg, "engine.vitriol.shim.enabled")
    if enabled is True:
        return Result("rule2-shim", False, False, "shim ENABLED — contract violation (REPORT-02 §2.1: stays flag-off)")
    return Result("rule2-shim", enabled is False, False, f"shim.enabled={enabled}")


def check_rule6_cert(cfg: dict) -> Result:
    req = dig(cfg, "engine.vitriol.cert_required")
    if req is not True:
        return Result("rule6-cert", False, False, "cert_required must be true (Rule 6)")
    return Result("rule6-cert", True, False, "cert gate armed")


def check_kill_switches(cfg: dict) -> Result:
    missing = [p for p in KILL_SWITCH_PATHS if not isinstance(dig(cfg, p), dict) or "enabled" not in dig(cfg, p)]
    if missing:
        return Result("rule15-kill", False, False, "stages without independent kill switch: " + ", ".join(missing))
    return Result("rule15-kill", True, False, f"{len(KILL_SWITCH_PATHS)} stages, all kill-switched")


def check_pipeline_order(cfg: dict) -> Result:
    order = dig(cfg, "coding.context_pipeline.order")
    want = ["clear", "compact", "compress"]
    if order != want:
        return Result("pipeline", False, False, f"order {order} != {want} (Rule 8: fixed)")
    return Result("pipeline", True, False, "clear -> compact -> compress")


def _profile_file(profile: str) -> Path | None:
    p = HOME / ".vitriol/profiles" / profile / "config"
    return p if p.exists() else None


def _parse_ini_profile(path: Path) -> dict[str, str]:
    """Vitriol profiles are INI-ish: collect key = value lines."""
    out: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        m = re.match(r"^([a-z_]+)\s*=\s*(.+?)\s*$", line.strip())
        if m:
            out[m.group(1)] = m.group(2)
    return out


def _profile_result(name: str) -> Result:
    """One profile: exists? model path fresh?"""
    pf = _profile_file(name)
    if pf is None:
        return Result(f"profile[{name}]", False, True, "profile dir missing in ~/.vitriol/profiles")
    model = _parse_ini_profile(pf).get("path", "")
    if model and not Path(model).exists():
        return Result(f"profile[{name}]", False, False, f"model path stale: {model} not on disk")
    return Result(f"profile[{name}]", True, False, f"model ok: {Path(model).name if model else '?'}")


def check_profiles(cfg: dict) -> list[Result]:
    """Referenced VITRIOL profiles exist; their model paths exist (stale = bug)."""
    names = [n for n in (dig(cfg, "engine.vitriol.profile"), dig(cfg, "engine.vitriol.smoke_profile"))
             if isinstance(n, str) and n]
    return [_profile_result(n) for n in names]


def _port_of(url: str) -> int | None:
    m = re.search(r"://[^/:]+:(\d+)", str(url or ""))
    return int(m.group(1)) if m else None


def _profile_port(name: str) -> int | None:
    """Port from a vitriol profile's [server] port key."""
    pf = _profile_file(name)
    if pf is None:
        return None
    return _num(_parse_ini_profile(pf).get("port", ""))


def _num(raw: str) -> int | None:
    try:
        return int(raw)
    except (TypeError, ValueError):
        return None


def _ports_from_components(cfg: dict, want: int) -> dict[str, int]:
    """Collect the engine port as each component believes it, unified included."""
    seen: dict[str, int] = {"unified": want}
    for pname in (dig(cfg, "engine.vitriol.profile"), dig(cfg, "engine.vitriol.smoke_profile")):
        if isinstance(pname, str) and pname:
            p = _profile_port(pname)
            if p is not None:
                seen[f"profile:{pname}"] = p
    lc_port = _port_of(_little_coder_base_url() or "")
    if lc_port:
        seen["little-coder/models.json"] = lc_port
    if _hermes_knows_port(want):
        seen["hermes/custom_providers"] = want
    return seen


def _little_coder_base_url() -> str | None:
    mc = HOME / ".config/little-coder/models.json"
    try:
        data = json.loads(mc.read_text())
        for prov in data.get("providers", {}).values():
            if prov.get("baseUrl"):
                return str(prov["baseUrl"])
    except (OSError, ValueError):
        return None
    return None


def _hermes_knows_port(want: int) -> bool:
    """True when a VITRIOL-named custom provider in hermes config uses want."""
    hc = HOME / ".hermes/config.yaml"
    try:
        hd = yaml.safe_load(hc.read_text()) or {}
    except (OSError, ValueError, yaml.YAMLError):
        return False
    for cp in hd.get("custom_providers", []) or []:
        if "VITRIOL" in str(cp.get("name", "")).upper() and _port_of(cp.get("base_url", "")) == want:
            return True
    return False


def check_port_parity(cfg: dict) -> list[Result]:
    """Port must match everywhere (AGENTS.md: stale entries are bugs)."""
    want = _port_of(dig(cfg, "engine.vitriol.endpoint") or "")
    if want is None:
        return [Result("parity-port", False, False, "unified config endpoint has no port")]
    seen = _ports_from_components(cfg, want)
    bad = {k: v for k, v in seen.items() if v != want}
    if bad:
        return [Result("parity-port", False, False, f"drift: {bad} != {want}")]
    return [Result("parity-port", True, False, f"port {want} consistent: {', '.join(sorted(seen))}")]


def engine_reachable(endpoint: str, timeout_s: float = 2.0) -> dict[str, Any]:
    """Live probe — status only, validate() stays static/offline-safe."""
    import urllib.request

    try:
        with urllib.request.urlopen(endpoint.rstrip("/") + "/health", timeout=timeout_s) as res:
            body = res.read().decode("utf-8", "replace")
        return {"reachable": True, "health": body[:80]}
    except OSError as e:
        return {"reachable": False, "health": str(e)[:80]}


def profile_inventory() -> list[dict[str, Any]]:
    """Every ~/.vitriol/profiles/*/config with model-path freshness."""
    root = HOME / ".vitriol/profiles"
    out: list[dict[str, Any]] = []
    if not root.exists():
        return out
    for pf in sorted(root.glob("*/config")):
        kv = _parse_ini_profile(pf)
        model = kv.get("path", "")
        proj = kv.get("mmproj", "")
        out.append({
            "profile": pf.parent.name,
            "model": Path(model).name if model else "",
            "model_exists": bool(model) and Path(model).exists(),
            # vision: a declared-but-missing projector is stale like a model
            "mmproj_exists": (not proj) or Path(proj).exists(),
        })
    return out


def _stale_result(item: dict, referenced: set) -> Result | None:
    """One inventory entry -> Result when its model is missing; None when fresh."""
    if not item["model_exists"]:
        ref = item["profile"] in referenced
        return Result(
            f"stale[{item['profile']}]", False, not ref,
            f"model {item['model'] or '?'} missing" + ("" if ref else " (unreferenced legacy profile)"),
        )
    if not item.get("mmproj_exists", True):
        return Result(f"stale[{item['profile']}]", False, False, "mmproj declared but missing (vision would 4xx)")
    return None


def check_all_profiles(cfg: dict) -> list[Result]:
    """P2.6: scan EVERY profile dir — referenced-stale FAILS, unreferenced WARNs.

    AGENTS.md names stale profiles a bug class (2026-08-28 double-stale);
    this makes the whole store visible to the gate, not just referenced ones."""
    referenced = {n for n in (dig(cfg, "engine.vitriol.profile"), dig(cfg, "engine.vitriol.smoke_profile")) if isinstance(n, str)}
    hits = [r for r in (_stale_result(it, referenced) for it in profile_inventory()) if r]
    return hits or [Result("stale-scan", True, False, "all profiles carry existing model files")]


def _parity_result(name: str, ids: set[str]) -> Result | None:
    """One referenced profile vs the scaffold's model ids. None = not applicable."""
    pf = _profile_file(name)
    if pf is None:
        return None
    mid = Path(_parse_ini_profile(pf).get("path", "")).name
    if not mid:
        return None
    if mid in ids:
        return Result(f"parity-modelsjson[{name}]", True, False, f"{mid} known to scaffold")
    return Result(f"parity-modelsjson[{name}]", False, False, f"{mid} NOT in little-coder models.json ids: {sorted(ids)}")


def check_models_json_parity(cfg: dict) -> list[Result]:
    """P2.7: each referenced profile's model filename must be an id little-coder knows.

    The 2026-08-28 lesson: configs went stale in THREE places after a model
    swap. Parity is checked statically (models.json ids, no engine needed)."""
    ids = _little_coder_ids()
    if ids is None:
        return [Result("parity-modelsjson", False, True, "little-coder models.json unreadable")]
    names = (dig(cfg, "engine.vitriol.profile"), dig(cfg, "engine.vitriol.smoke_profile"))
    refs = [r for r in (_parity_result(str(n), ids) for n in names if isinstance(n, str) and n) if r]
    return refs


def _little_coder_ids() -> set[str] | None:
    mc = HOME / ".config/little-coder/models.json"
    try:
        data = json.loads(mc.read_text())
        from itertools import chain

        groups = (prov.get("models") or [] for prov in data.get("providers", {}).values())
        return {str(m["id"]) for m in chain.from_iterable(groups) if isinstance(m, dict) and m.get("id")}
    except (OSError, ValueError, TypeError):
        return None


def validate(config_path: Path) -> Report:
    """Full gate over the unified config + component files."""
    rep = Report()
    try:
        cfg = load_unified(config_path)
    except (OSError, ValueError, yaml.YAMLError) as e:
        rep.results.append(Result("structure", False, False, f"cannot load {config_path}: {str(e)[:200]}"))
        return rep
    rep.results.append(check_structure(cfg))
    rep.results.append(check_rule2_shim(cfg))
    rep.results.append(check_rule6_cert(cfg))
    rep.results.append(check_kill_switches(cfg))
    rep.results.append(check_pipeline_order(cfg))
    rep.results.extend(check_profiles(cfg))
    rep.results.extend(check_all_profiles(cfg))
    rep.results.append(check_perms_mirror(cfg))
    rep.results.append(check_dispatch_roots(cfg))
    rep.results.extend(check_models_json_parity(cfg))
    rep.results.extend(check_port_parity(cfg))
    return rep


def render(rep: Report, as_json: bool = False) -> str:
    if as_json:
        return json.dumps(
            {"results": [{"check": r.name, "ok": r.ok, "severity": "warn" if r.soft else ("fail" if not r.ok else "pass"), "detail": r.detail} for r in rep.results],
             "exit": rep.exit_code},
            indent=1,
        )
    lines = []
    for r in rep.results:
        mark = "PASS" if r.ok else ("WARN" if r.soft else "FAIL")
        lines.append(f"[{mark}] {r.name:28s} {r.detail}")
    tail = f"{len(rep.results) - len(rep.failed) - len(rep.warned)} pass / {len(rep.warned)} warn / {len(rep.failed)} fail"
    lines.append("─" * 8 + " " + tail)
    return "\n".join(lines)


def check_dispatch_roots(cfg: dict) -> Result:
    """Audit F11: sub-coders have near-unrestricted shell; approved roots
    bound the spawn surface. Unset/empty is a WARN today (local threat
    model) and MUST be non-empty before any gateway exposure."""
    roots = dig(cfg, "safety.dispatch_roots")
    if isinstance(roots, list) and roots:
        return Result("dispatch-roots", True, False, f"{len(roots)} approved root(s): {[str(r) for r in roots][:5]}")
    return Result("dispatch-roots", False, True, "safety.dispatch_roots empty — sub-coder spawn unrestricted (required before any gateway exposure)")


def check_perms_mirror(cfg: dict) -> Result:
    """safety.permissions must have a FRESH JSON mirror (permissions.json).

    The scaffold runtime consumes the mirror; an absent or stale mirror means
    the declared policy is not enforced — drift, so FAIL (never warn)."""
    from tris_lib import perms_snapshot, perms_snapshot_path

    safety = cfg.get("safety", {}) if isinstance(cfg.get("safety"), dict) else {}
    rules = safety.get("permissions") or []
    if not rules:
        return Result("perms-mirror", True, False, "no permissions declared (nothing to mirror)")
    want = perms_snapshot(cfg)
    path = perms_snapshot_path()
    try:
        have = json.loads(path.read_text())
    except (OSError, ValueError):
        return Result("perms-mirror", False, False, f"mirror missing: {path} (run: tris perms-sync)")
    if have.get("source_hash") != want["source_hash"]:
        return Result("perms-mirror", False, False, "mirror STALE vs config.yaml (run: tris perms-sync)")
    return Result("perms-mirror", True, False, f"{len(rules)} rules mirrored ({want['source_hash']})")
