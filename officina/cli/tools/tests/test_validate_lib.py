"""Tests for the unified-config validator (step 23).

Run: ~/venvs/tris/bin/python -m pytest tools/tests -q   (from the repo root)
"""

import sys
from pathlib import Path

import pytest
import yaml

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import json

from validate_lib import KILL_SWITCH_PATHS, dig, engine_reachable, render, validate  # noqa: E402

GOOD = {
    "engine": {"vitriol": {
        "endpoint": "http://127.0.0.1:8279",
        "profile": "does-not-exist-profile",   # soft warn only
        "smoke_profile": "",
        "shim": {"enabled": False},
        "cert_required": True,
    }},
    "coding": {
        "context_pipeline": {
            "order": ["clear", "compact", "compress"],
            "clear": {
                "tool_result_clearer": {"enabled": True},
                "rtk_output": {"enabled": True},
            },
            "compact": {
                "async_compaction": {"enabled": True},
                "batch_aware": {"enabled": False},
            },
            "compress": {
                "llmlingua2": {"enabled": False},
                "caveman_rules": {"enabled": False},
            },
        },
        "injection": {
            "repo_map": {"enabled": True},
            "task_state": {"enabled": True},
            "diagnostics_loop": {"enabled": True},
            "snapshot": {"enabled": False},
            "vitriol_checkpoint": {"enabled": True},
            "hermes_bridge": {"enabled": True},
            "rewoo_dispatch": {"enabled": False},
        },
    },
    "gateway": {"hermes": {
        "memory_extractor": {"enabled": False},
        "context_relay": {"enabled": False},
        "injection_guard": {"enabled": False},
    }},
    "safety": {"approval_required": ["force_push"]},
}


@pytest.fixture()
def good_file(tmp_path):
    def make(mutate=None):
        cfg = yaml.safe_load(yaml.safe_dump(GOOD))
        if mutate:
            mutate(cfg)
        p = tmp_path / "config.yaml"
        p.write_text(yaml.safe_dump(cfg))
        return p
    return make


def names(rep):
    return {r.name: r for r in rep.results}


class TestHappyPath:
    def test_no_hard_failures(self, good_file):
        rep = validate(good_file())
        assert rep.exit_code == 0, [r.detail for r in rep.failed]

    def test_kill_switch_count_matches_inventory(self, good_file):
        rep = validate(good_file())
        assert "16 stages" in names(rep)["rule15-kill"].detail
        assert len(KILL_SWITCH_PATHS) == 16

    def test_dispatch_roots_empty_warns(self, good_file):
        rep = validate(good_file())
        r = names(rep)["dispatch-roots"]
        assert not r.ok and r.soft  # WARN: unrestricted, required pre-gateway

    def test_dispatch_roots_set_passes(self, good_file):
        rep = validate(good_file(lambda c: c["safety"].__setitem__("dispatch_roots", ["/tmp/opencode"])))
        r = names(rep)["dispatch-roots"]
        assert r.ok and "1 approved" in r.detail


class TestContractViolations:
    def test_shim_enabled_is_fail(self, good_file):
        rep = validate(good_file(lambda c: c["engine"]["vitriol"]["shim"].__setitem__("enabled", True)))
        r = names(rep)["rule2-shim"]
        assert not r.ok and not r.soft  # FAIL, never warn

    def test_cert_off_is_fail(self, good_file):
        rep = validate(good_file(lambda c: c["engine"]["vitriol"].__setitem__("cert_required", False)))
        assert names(rep)["rule6-cert"].ok is False

    def test_reordered_pipeline_is_fail(self, good_file):
        rep = validate(good_file(lambda c: c["coding"]["context_pipeline"].__setitem__("order", ["compress", "clear", "compact"])))
        assert names(rep)["pipeline"].ok is False

    def test_missing_kill_switch_is_fail(self, good_file):
        def mut(c):
            del c["coding"]["context_pipeline"]["clear"]["rtk_output"]["enabled"]
        rep = validate(good_file(mut))
        r = names(rep)["rule15-kill"]
        assert not r.ok and "rtk_output" in r.detail

    def test_missing_required_key_is_fail(self, good_file):
        rep = validate(good_file(lambda c: c["safety"].__setitem__("approval_required", None) or c["safety"].pop("approval_required")))
        assert names(rep)["structure"].ok is False

    def test_unparseable_config_is_fail(self, tmp_path):
        p = tmp_path / "config.yaml"
        p.write_text("engine: [unclosed")
        rep = validate(p)
        assert rep.exit_code == 1 and "cannot load" in rep.failed[0].detail


class TestProfileDrift:
    def test_stale_model_path_fails(self, good_file, tmp_path, monkeypatch):
        prof = tmp_path / ".vitriol/profiles/p1/config"
        prof.parent.mkdir(parents=True)
        prof.write_text("[model]\npath = /nonexistent/model.gguf\n[server]\nport = 9999\n")
        monkeypatch.setattr("validate_lib.HOME", tmp_path)
        rep = validate(good_file(lambda c: c["engine"]["vitriol"].__setitem__("profile", "p1")))
        r = names(rep)["profile[p1]"]
        assert not r.ok and "stale" in r.detail

    def test_port_drift_fails(self, good_file, tmp_path, monkeypatch):
        prof = tmp_path / ".vitriol/profiles/p2/config"
        prof.parent.mkdir(parents=True)
        prof.write_text("[model]\npath = /tmp/whatever\n[server]\nhost = 127.0.0.1\nport = 1234\n")
        monkeypatch.setattr("validate_lib.HOME", tmp_path)
        rep = validate(good_file(lambda c: c["engine"]["vitriol"].__setitem__("profile", "p2")))
        r = names(rep)["parity-port"]
        assert not r.ok and "drift" in r.detail




class TestPermsMirrorCheck:
    def test_missing_mirror_fails(self, good_file, tmp_path, monkeypatch):
        def mut(c):
            c["safety"]["permissions"] = [{"tool": "write", "path": "**/.env", "action": "deny"}]
        monkeypatch.setenv("TRIS_PERMS_FILE", str(tmp_path / "absent.json"))
        rep = validate(good_file(mut))
        r = [x for x in rep.results if x.name == "perms-mirror"][0]
        assert not r.ok and "missing" in r.detail

    def test_stale_mirror_fails_fresh_passes(self, good_file, tmp_path, monkeypatch):
        import json as _json
        import tris_lib as tl_local

        def mut(c):
            c["safety"]["permissions"] = [{"tool": "write", "path": "**/.env", "action": "deny"}]
        pf = tmp_path / "permissions.json"
        monkeypatch.setenv("TRIS_PERMS_FILE", str(pf))
        cfg = yaml.safe_load(good_file(mut).read_text())
        pf.write_text(_json.dumps({**tl_local.perms_snapshot(cfg), "source_hash": "deadbeef"}))
        r = [x for x in validate(good_file(mut)).results if x.name == "perms-mirror"][0]
        assert not r.ok and "STALE" in r.detail
        pf.write_text(_json.dumps(tl_local.perms_snapshot(cfg)))
        assert [x for x in validate(good_file(mut)).results if x.name == "perms-mirror"][0].ok


class TestStoreWideHygiene:
    def test_unreferenced_stale_profile_warns(self, good_file, tmp_path, monkeypatch):
        dead = tmp_path / "gone.gguf"
        prof = tmp_path / ".vitriol/profiles/legacy/config"
        prof.parent.mkdir(parents=True)
        prof.write_text(f"[model]\npath = {dead}\n[server]\nport = 8279\n")
        monkeypatch.setattr("validate_lib.HOME", tmp_path)
        rep = validate(good_file())
        r = [x for x in rep.results if x.name == "stale[legacy]"]
        assert r and not r[0].ok and r[0].soft  # WARN not FAIL
        assert rep.exit_code == 0

    def test_referenced_stale_profile_fails(self, good_file, tmp_path, monkeypatch):
        prof = tmp_path / ".vitriol/profiles/p1/config"
        prof.parent.mkdir(parents=True)
        prof.write_text("[model]\npath = /nope/x.gguf\n[server]\nport = 8279\n")
        monkeypatch.setattr("validate_lib.HOME", tmp_path)
        rep = validate(good_file(lambda c: c["engine"]["vitriol"].__setitem__("profile", "p1")))
        r = [x for x in rep.results if x.name == "stale[p1]"][0]
        assert not r.ok and not r.soft  # FAIL
        assert rep.exit_code == 1

    def test_modelsjson_parity_pass_and_fail(self, good_file, tmp_path, monkeypatch):
        prof = tmp_path / ".vitriol/profiles/px/config"
        prof.parent.mkdir(parents=True)
        prof.write_text("[model]\npath = /somewhere/M.gguf\n[server]\nport = 8279\n")
        lc = tmp_path / ".config/little-coder"
        lc.mkdir(parents=True)
        (lc / "models.json").write_text(json.dumps(
            {"providers": {"llamacpp": {"baseUrl": "http://127.0.0.1:8279/v1",
                                         "models": [{"id": "OTHER.gguf"}]}}}))
        monkeypatch.setattr("validate_lib.HOME", tmp_path)
        rep = validate(good_file(lambda c: c["engine"]["vitriol"].__setitem__("profile", "px")))
        r = [x for x in rep.results if x.name == "parity-modelsjson[px]"][0]
        assert not r.ok and "M.gguf NOT in" in r.detail
        assert rep.exit_code == 1

    def test_engine_probe_down_never_raises(self):
        out = engine_reachable("http://127.0.0.1:1", timeout_s=0.3)
        assert out["reachable"] is False and out["health"]


def test_missing_mmproj_fails_even_when_model_present():
    item = {"profile": "px", "model": "m.gguf", "model_exists": True, "mmproj_exists": False}
    r = _stale(item, {"px"})
    assert r is not None and not r.ok and not r.soft and "mmproj" in r.detail


def test_fresh_model_without_mmproj_key_is_clean():
    item = {"profile": "px", "model": "m.gguf", "model_exists": True, "mmproj_exists": True}
    assert _stale(item, set()) is None


def _stale(item, ref):
    from validate_lib import _stale_result
    return _stale_result(item, ref)


class TestRendering:
    def test_json_shape(self, good_file):
        import json
        out = render(validate(good_file()), as_json=True)
        data = json.loads(out)
        assert data["exit"] == 0 and len(data["results"]) >= 6

    def test_text_marks(self, good_file):
        out = render(validate(good_file()))
        assert "[PASS]" in out and "8 pass" in out or "pass /" in out


def test_dig_helper():
    assert dig({"a": {"b": 1}}, "a.b") == 1
    assert dig({}, "a.b") is None
    assert dig({"a": 1}, "a.b") is None  # non-dict midpath
