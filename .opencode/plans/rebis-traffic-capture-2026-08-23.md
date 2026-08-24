# REBIS traffic capture — inter-model message log

**Date:** 2026-08-23 23:30
**Status:** implementing
**Vision:** see what the models send and stream to each other — Luna's
drafts, Sol's verdicts and corrections — live in the TUI, archived for
debugging and training.

## Design

**Capture point:** Mercury (`gateway_turn`) — every inter-model exchange
passes through it already.

**Storage:** `~/.vitriol/distill/traffic.jsonl` (local-only policy), one
JSONL per exchange, tail-8KB of prompt/response bodies, rotation at boot +
20 MiB inline cap (→ `.jsonl.1`).

| kind | from → to | body |
|---|---|---|
| draft | luna → sol | Mandatum-context prompt + streamed draft |
| draft_retry | luna → sol | corrective nudge round |
| audit | sol → luna | full audit prompt + constrained verdict |
| correction | sol → luna | corrective orders + Sol-authored replacement |
| warm | sol → sol | metadata-only (body excluded; `REBIS_TRAFFIC_WARM=1` opts in) |

**TUI:** LOGS tab gains a TRAFFIC chip (7 sources, keys 1–7); concise mode
renders JSONL heads (ts/kind/from/to first — truncation shows the summary
naturally); verbose shows full bodies. Warm meta-lines hidden in concise.

## Units

1. `traffic_log()` helper (rotation + warm toggle) + emission at exchange
   points in `gateway_turn`
2. Boot rotation in `rebis-gateway.sh`
3. TUI: config/poller/model/LogSource::Traffic + chips + keybind 7
4. Selftests: JSONL validity, rotation, warm toggle

## Progress log

- 2026-08-23 23:30 — plan written; implementing.

## Progress log (cont.)

- 2026-08-23 23:55 — **TRAFFIC CAPTURE LIVE.** Validated end-to-end:
  kickoff turn produced `draft luna→sol 94→739 10.22s` + `audit sol→luna
  1398→514 18.94s` in traffic.jsonl; TUI TRAFFIC chip renders formatted
  summary lines (concise) with full bodies on v. Warm prefills excluded
  by default, REBIS_TRAFFIC_WARM=1 opts in (metadata-only). Rotation:
  boot + 20 MiB inline cap.
- Pipeline draft emission wired (was fast-path only).
- Remaining: REBIS event stream ↔ traffic correlation (v2); REBIS_TRAFFIC_WARM
  surface in CONFIG pane (minor).
