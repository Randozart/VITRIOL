# Command renames (clean break) + /fx, liveliness contract, watchdog card

- Date: 2026-11-13
- Status: in progress
- Decisions: clean break, NO aliases, old names deleted; /plan and
  /deep-research unchanged; glimmer stays decorative; /fx only (kill
  /fire + /glimmer); watchdog fields via a facts file.

## Batch A — command renames (old names deleted)

| Old | New | Owner |
|---|---|---|
| `/why` | `/route-why` | resilient-default |
| `/overflow on\|off` | `/route-overflow on\|off` | adaptive-router |
| `/overflow-catalog` | **deleted** (dup of `/free`) | adaptive-router |
| `/profile` | `/model-profile` | resilient-default |
| `/coupling` | `/model-swap` | coupling |
| `/model-handover` | `/model-phase` | phase-model |
| `/phase-models` | `/model-phases` | phase-model |
| `/providers` | `/provider-health` | caduceus |
| `/keys` | `/provider-keys` | caduceus |
| `/caduceus-add` | `/provider-add` | caduceus |
| ext `/cost` | `/budget` | resilient-default |
| `/import` | `/import-tasks` | session-panel |
| TUI `/opencode` | `/import-opencode` | TUI local |
| `/session-name` | **deleted** (whole ext; TUI `/rename` survives) | session-name |
| `/panel` | **deleted** (TUI `/sidebar` survives) | session-panel |
| `/fire`, `/glimmer` | **deleted** (superseded by `/fx`) | TUI local |

Uniqueness guard test: scans `.pi/extensions/**/index.ts` for
`registerCommand("…")` and `LOCAL_COMMANDS` in the TUI source; fails on
any duplicate name (this is what would have caught the `/cost` shadow).

## Batch B — /fx + liveliness + watchdog card

### B1 Liveliness contract (fire only; glimmer decorative)
- `state.last_activity: Option<Instant>` updated on streamed text, tool
  start/update/end, agent start/end, prompt submit.
- `Liveliness::{Active(source), Stalled(secs)}` (active window ~10 s).
- Stalled ⇒ `fire_target = 0` (0.33 s decay, flames genuinely vanish).
- Redraw cadence: 90 ms while Active, empty screen (glimmer), or fire
  lit; otherwise 500 ms heartbeat.

### B2 `/fx` modal
- Rows: fire on/off · voice (prismatic|emerald|alchemy) · tint ·
  glimmer (shimmer|breathe|twinkle|off).
- Live preview: fire strip + glimmer sample, animated in-modal; fire
  preview disappears when Stalled.
- Status chip: ACTIVE · <source> | STALLED — fire hidden.
- Persists via the existing tui-fire / tui-glimmer files (env precedence
  preserved). `/fire` and `/glimmer` commands removed.

### B3 Watchdog card
- `hang-watchdog` writes `~/.vitriol/officina/watchdog.json`
  (`version, kind, command, elapsed_s, stalled_s, verdict, reason, ts,
  next_check_at, phase`), beside the notify.
- Countdown in the running-tool row (owner request): while a tool runs and
  the facts are fresh, the row shows `working… · watchdog 21s` — the
  seconds until the next watchdog check (soft notice / judge / re-judge),
  so silence has visible, honest feedback.
- TUI renders a structured card: verdict chip (ALIVE/HUNG/JUDGING),
  cmd/elapsed/stalled/reason rows, `esc aborts · no auto-kill` footer.
  Raw-text fallback when unreadable.

## Invariants

- I1 fire never animates without evidence (stream/tool CPU/recent
  activity); glimmer unchanged (decorative on empty screen).
- I2 `/fx` writes only the two existing persistence files; env
  precedence unchanged.
- I3 watchdog card never loses a message (raw fallback).
- I4 every deleted command's replacement is registered in the same
  commit; no aliases.
- I5 no duplicate command names (guard test).

## Gates

- Batch A: `npm test` + `npm run typecheck` + `cargo test`/fmt (TUI
  rename) + rebuild/install.
- B1/B2: cargo test + fmt + release build + install.
- B3: npm test + tsc (writer) + cargo test (card).
