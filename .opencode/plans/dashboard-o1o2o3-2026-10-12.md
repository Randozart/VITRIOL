# Dashboard Telemetry: O1 Acceptance + O2 VRAM + O3 Cache-Hit

**Date**: 2026-10-12
**Status**: APPROVED (owner: "do O1-3, potentially O4, skip O0; show it in the
VITRIOL dashboard, not Officina")
**Target surface**: `vitriol-tui` (Rust/Ratatui, `vitriol tui`) — the owner's
telemetry surface. Officina sidebar NOT touched. O4 (`vitriol-tap` proxy)
deferred ("potentially").

## Revised scope after code read (2026-10-12 inventory)

| item | original plan | reality found | work left |
|---|---|---|---|
| O1 acceptance | parse `/metrics` spec counters, render acc | `ui.rs:243-247` already renders `DraftSnapshot` — but fed from `/props` `draft`, which our engine never serves (vestigial; `/props` has no draft key). `/metrics` DOES carry `spec_decode_num_{draft,accepted,drafts}_tokens_total` | add 3 counters to `MetricsTotals` + `parse_metrics_text`; derive `draft` from totals when `/props` silent; bonus: `kv_ejected_total` in the totals line |
| O2 VRAM | extend nvidia-smi query | **already shipped**: `nvidia.rs` queries `memory.used,total`; `ui.rs:403-416` + `:682-694` render per-GPU GiB + gauge | verify live; no code |
| O3 cache-hit | fix engine counter + render hit% | counter `prompt_tokens_cached_total` read 0; plumbing (`add_prompt_cached` `server-common.h:513` → `to_metrics` `server-task.cpp:1538`) reads intact; only call site is `server-context.cpp:3751` (prefix-keep path) | **probe-first**: double identical request; if counter moves, the AGENTS "broken" note is stale or restore-path-only → then investigate slot-restore bypass; fix only what a probe proves broken. TUI: add `prompt_tokens_cached_total` + render `cache N% = cached/(cached+processed)` |

## Changes

### Stage 1 — vitriol-tui (no engine dependency)

- `model.rs`: `MetricsTotals` += `prompt_tokens_cached_total`,
  `spec_draft_total`, `spec_accepted_total`, `spec_draft_steps_total`,
  `kv_ejected_total`.
- `poller.rs`: `parse_metrics_text` += the five names; `poll_gen` reorders
  totals-before-draft and falls back:
  `draft = poll_draft(..) or DraftSnapshot{spec counters}` (keeps working on
  servers that do serve `/props` draft).
- `ui.rs` totals line (~:287): append `· cache N%` and `· ⤓ Nk`.
- Tests: extend `metrics_text_extracts_counters_and_gauges`; new fallback
  derivation test.
- Verify: `cargo test` + `cargo build --release`; live against the unit.

### Stage 2 — engine O3 (only if probe proves broken)

- Probe: two identical `/completion` calls → `/metrics` delta.
- If broken: trace why the `:3751` path is skipped (or why `n_past` is 0),
  fix minimally, rebuild `llama-server`, `systemctl restart
  vitriol-server.service`, fingerprint diff (old `12:39` line vs new), re-probe.
- Note: engine was SIGKILL'd at 14:00:40 after a SIGTERM timeout (wedge:
  0% GPU, 17% CPU, accept queue full, 2.9 GiB swap) — unrelated pre-existing
  issue, reported to owner separately.

## Non-goals

- Officina sidebar (owner directive).
- Per-position acceptance series (stretch, not needed for the gating answer).
- O4 proxy — revisit after O1–O3 land.

## Provenance

Dashboard polls cached `/metrics` only (non-blocking by 2026-09-04 design) —
no new engine load. Engine changes (if any) ride the standard fingerprint
discipline.
