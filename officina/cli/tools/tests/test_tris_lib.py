"""Tests for the tris data-plane helpers (Round 4 T1).

Run: ~/venvs/tris/bin/python -m pytest tools/tests -q   (repo root)
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import tris_lib as tl  # noqa: E402

FORK_METRICS = """# HELP llamacpp:prompt_tokens_total Number of prompt tokens processed.
llamacpp:prompt_tokens_total 17500
llamacpp:n_decode_total 812
llamacpp:n_tokens_max 4096
garbage line without number
"""


class TestMetrics:
    def test_parses_fork_names(self, monkeypatch):
        import urllib.request

        class Resp:
            def read(self_):
                return FORK_METRICS.encode()

            def __enter__(self_):
                return self_

            def __exit__(self_, *a):
                return False

        monkeypatch.setattr(urllib.request, "urlopen", lambda *a, **k: Resp())
        snap = tl.metrics_snapshot("http://x")
        assert snap == {"prompt_tokens": 17500.0, "generated_tokens": 812.0}

    def test_unreachable_returns_none(self):
        assert tl.metrics_snapshot("http://127.0.0.1:1") is None

    def test_diff_is_delta_never_guess(self):
        d = tl.diff_metrics({"prompt_tokens": 100, "generated_tokens": 10},
                            {"prompt_tokens": 250, "generated_tokens": 55})
        assert d == {"prompt_tokens": 150, "generated_tokens": 45}
        assert tl.diff_metrics(None, {"a": 1}) == {}
        assert tl.diff_metrics({"a": 5}, {"a": 2}) == {"a": 0}  # counter reset clamps at 0


class TestEventLedger:
    def test_append_tail_roundtrip(self, tmp_path, monkeypatch):
        monkeypatch.setattr(tl, "STATE_DIR", tmp_path)
        tl.emit_event("lc-clearer", "cleared", "3 stubs", freed_tokens=1200, turn=7)
        tl.emit_event("not-a-source", "x")  # unknown src rejected
        tl.emit_event("vb-gate", "gate-block", "kv 95%")
        evs = tl.tail_events()
        assert [e["src"] for e in evs] == ["lc-clearer", "vb-gate"]
        assert evs[0]["freed_tokens"] == 1200

    def test_event_failure_never_raises(self, tmp_path, monkeypatch):
        monkeypatch.setattr(tl, "STATE_DIR", Path("/proc/definitely-not-writable"))
        tl.emit_event("tris", "x")  # must swallow
        tl.append_ledger({"a": 1})  # must swallow

    def test_ledger_roundtrip_and_corrupt_lines(self, tmp_path, monkeypatch):
        monkeypatch.setattr(tl, "STATE_DIR", tmp_path)
        tl.append_ledger({"task": "one", "success": True})
        (tmp_path / "ledger.jsonl").write_text('{"task": "corrupt"\n')
        assert tl.read_ledger() == []
        tl.append_ledger({"task": "two"})
        assert [r["task"] for r in tl.read_ledger()] == ["two"]


class TestFingerprint:
    def test_journal_preferred_over_log(self, tmp_path, monkeypatch):
        (tmp_path / "serve.log").write_text("VITRIOL-FINGERPRINT model=OLD.gguf\n")
        monkeypatch.setattr(tl, "_journal", lambda cmd: "x\nVITRIOL-FINGERPRINT model=NEW.gguf ts=none\n")
        assert "NEW.gguf" in tl.fingerprint_from_log(str(tmp_path / "serve.log"))

    def test_falls_back_to_log_when_no_unit(self, tmp_path, monkeypatch):
        (tmp_path / "serve.log").write_text("noise\nVITRIOL-FINGERPRINT model=LOGONLY.gguf\n")
        monkeypatch.setattr(tl, "_journal", lambda cmd: "")
        assert "LOGONLY" in tl.fingerprint_from_log(str(tmp_path / "serve.log"))


class TestBadge:
    def test_dev_until_dual_gpu_27b(self):
        assert tl.model_certified("model=Qwen3.8-9B-Q8_0.gguf ts=none") is False
        assert tl.model_certified("model=Qwen3.8-27B-Q3_K_M.gguf ts=27,9") is True
        assert tl.model_certified("model=Qwen3.8-27B-Q3_K_M.gguf ts=none") is False


class TestBudget:
    CFG = """
coding:
  injection:
    repo_map: { enabled: true, budget: 1000 }
    task_state: { enabled: true, budget: 200 }
  context_pipeline:
    order: [clear, compact, compress]
"""

    def test_alloc_rows(self):
        rows = tl.budget_table(self.CFG)
        assert {"item": "coding.injection.repo_map", "budget": 1000} in rows
        assert {r["item"] for r in rows} == {"coding.injection.repo_map", "coding.injection.task_state"}

    def test_broken_yaml_returns_empty_not_crash(self):
        assert tl.budget_table("a: [unclosed") == []


def test_live_real_config_budget_rows():
    cfg = Path.home() / ".config/trismegistus/config.yaml"  # detached live config (fold-in 2026-08-31)
    rows = tl.budget_table(cfg.read_text())
    assert len(rows) >= 7
    assert all(r["budget"] > 0 for r in rows)


class TestPermsMirror:
    def test_snapshot_shape_and_hash_stability(self):
        cfg = {"safety": {"default_action": "ask", "permissions": [
            {"tool": "Write", "path": "**/.env", "action": "deny"}]}}
        a, b = tl.perms_snapshot(cfg), tl.perms_snapshot(cfg)
        assert a["default_action"] == "ask"
        assert a["rules"] == [{"tool": "write", "pattern": "**/.env", "action": "deny"}]
        assert a["source_hash"] == b["source_hash"] and len(a["source_hash"]) == 16

    def test_hash_changes_on_policy_edit(self):
        base = {"safety": {"default_action": "allow", "permissions": [{"tool": "write", "path": "a", "action": "deny"}]}}
        moved = {"safety": {"default_action": "allow", "permissions": [{"tool": "write", "path": "a", "action": "ask"}]}}
        assert tl.perms_snapshot(base)["source_hash"] != tl.perms_snapshot(moved)["source_hash"]

    def test_bad_default_falls_back_allow(self):
        assert tl.perms_snapshot({"safety": {"default_action": "yolo", "permissions": []}})["default_action"] == "allow"

    def test_sync_writes_readable_mirror(self, tmp_path, monkeypatch):
        monkeypatch.setenv("TRIS_PERMS_FILE", str(tmp_path / "permissions.json"))
        path = tl.sync_perms({"safety": {"permissions": [{"tool": "write", "path": "**/.env", "action": "deny"}]}})
        snap = json.loads(path.read_text())
        assert snap["rules"][0]["pattern"] == "**/.env" and snap["source_hash"]


class TestLanes:
    """M8 MVP (2026-08-31): read-only lane snapshot; /props truth, not guesses."""

    def test_down_lane_reported_honestly(self):
        snap = tl.lane_snapshot({"ghost": "http://127.0.0.1:1"}, timeout_s=0.5)
        assert snap["lanes"]["ghost"] == {"endpoint": "http://127.0.0.1:1", "status": "down"}

    def test_up_lane_reports_props_truth(self, monkeypatch):
        calls = []

        def fake_get(url, timeout_s):
            calls.append(url)
            if url.endswith("/health"):
                return {"status": "ok"}
            return {
                "default_generation_settings": {"model": "Qwen3.8-27B-Q3_K_M", "n_ctx": 81920},
                "modalities": {"vision": True},
            }

        monkeypatch.setattr(tl, "_get_json", fake_get)
        monkeypatch.setattr(tl, "fingerprint_from_log", lambda *a, **k: "FP-LINE")
        snap = tl.lane_snapshot({"master": "http://127.0.0.1:8279/"})
        lane = snap["lanes"]["master"]
        assert lane["status"] == "ok" and lane["model"] == "Qwen3.8-27B-Q3_K_M"
        assert lane["n_ctx"] == 81920 and lane["vision"] is True
        assert snap["fingerprint"] == "FP-LINE"
        assert calls == ["http://127.0.0.1:8279/health", "http://127.0.0.1:8279/props"]
