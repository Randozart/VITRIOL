# TRIS — the custom Trismegistus experience

**2026-08-29** (build plan approved: "full custom experience using the best
available options"; ratatui/Rust cockpit; `tris` entry point).

## Principle

Custom *experience*, best-of-breed *plumbing*. Agent loops stay rented
(Rules 2 & 9: pi owns coding sessions, Hermes owns chat, VITRIOL owns
KV). What becomes ours: the single entry point, the cockpit screen, the
budget law made visible, and the measurement ledger.

## Architecture

```
                    ┌─────────────────────────────────────┐
                    │  tris (python CLI, bin/trismegistus) │
                    │  up/down/smoke/status/validate       │  ← wraps existing
                    │  code → little-coder TUI (guaranteed env)
                    │  chat → hermes CLI (vitriol provider) │
                    │  go "<task>" → bridge dispatch + ledger
                    │  budget → allocation vs measured      │
                    │  watch → tris-watch (RUST, below)     │
                    └───────────────┬─────────────────────┘
        shared data plane (files + HTTP — every producer already exists)
                    ┌───────────────┴──────────────────────┐
                    │ engine: /health /slots /metrics (HTTP)│
                    │ events: ~/.local/state/trismegistus/  │
                    │   events.jsonl  (stage firings)       │
                    │   ledger.jsonl  (per-task records)    │
                    │ tasks: <project>/.pi/tasks/*.json     │
                    │ law:   config/config.yaml (budget §R2.8)
                    │ ckpts: .pi/ckpt/*.json + slot files   │
                    └───────────────┬──────────────────────┘
                    ┌───────────────┴──────────────────────┐
                    │ tris-watch (Rust, ratatui 0.29,      │  ← tui/ crate,
                    │ Vitriolum sibling of vitriol-tui)    │    house pattern
                    │ panes: BUDGET PIPELINE TASKS         │
                    │        DISPATCH LAW                  │
                    └──────────────────────────────────────┘
```

## Event & ledger schemas (single source: this file)

`events.jsonl` — one line per pipeline stage firing (best-effort append;
producers MUST NOT block or fail a turn on event trouble):

```json
{"ts": 1787000000.1, "src": "lc-clearer|lc-rtk|lc-ckpt|lc-relay|lc-tasks|vb-gate|vb-dispatch",
 "ev": "cleared|reduced|saved|restored|injected|updated|gate-block|spawn",
 "freed_tokens": 1234, "detail": "…", "turn": 12, "session": "stem"}
```

`ledger.jsonl` — one record per task (the R3 measurement spine; every line
carries the fingerprint and a DEV/CERTIFIED badge — Rule 4).
`t_s` = generated-tokens / WALL seconds = effective task throughput
(startup, prefill, thinking included) — deliberately NOT the engine's
instantaneous decode rate; both appear post-R3 (prompt/predicted seconds
counters give the split). Never quote one as the other.

```json
{"ts": …, "fingerprint": "VITRIOL-FINGERPRINT …", "certified": false,
 "task": "…", "model": "…", "tok_in": …, "tok_out": …, "cache_read": …,
 "api_calls": …, "accounting": "session-usage|hermes-usage-file|metrics-delta-fallback",
 "sanity_metrics": {…}, "t_s": …,
 "gate": {"kv_fill_pct": null, "allowed": true}, "exit": 0}
```

**Accounting truth hierarchy (audit F3, 2026-08-30):** `tok_in/tok_out/
cache_read/api_calls` come from the CONSUMER (pi per-message session usage
via the dispatch `--session-dir`; or `tris ledger-ingest` of a Hermes
`--usage-file` report). Engine `/metrics` deltas ride along ONLY as
`sanity_metrics` — they are polluted by any concurrent engine traffic and
must never be quoted as task cost. `accounting` names the source; a
`metrics-delta-fallback` row is flagged, never silently equal.

## Panes (tris-watch)

| Pane | Source | Content |
|------|--------|---------|
| BUDGET | config allocation + ledger + /metrics | §R2.8 rows vs measured; cache-hit %; tok/task |
| PIPELINE | events.jsonl (tail) | last N stage firings, tokens freed |
| TASKS | newest .pi/tasks/*.json (per project) | checklist render (same marks as task-state) |
| DISPATCH | /slots + gate events + semaphore | active sub-coders, verdicts, checkpoint timeline |
| LAW | trismegistus validate (subprocess, cached) | gate status, 19 WARNs, dark stages, fingerprint + DEV/CERT badge |

## Phases

- **T1** `tris` command tree + event/ledger helpers (python) + `tris go` — day
- **T2** tris-watch ratatui cockpit (data layer unit-tested; `--json` dump
  for headless checks) — 1-2 days
- **T3** control from cockpit (dispatch key, checkpoint/rewind, stage toggles)
- **T4** post-reboot: `tris certify` (VITRIOL cert suite) flips badges; R3.5b
  sweep view reuses vitriol-calibrate sweep controller

Kill switches: events/ledger appenders honor their stage's switch
(switch off = silence, no extra failure surface). `tris watch` reads only —
writes nothing (Rule 7: the observer must never touch the observed cache).
