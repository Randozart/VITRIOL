---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T13:16:28+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "90a1c71dfe3dc9b25f607cd8edd8fdb133f3cf2c"
supersedes: []
superseded_by: []
related: []
---

# vitriol-tui-two-tabs

## Purpose

Make Dashboard and Logs the operator console and fix Aether background and stale wiring.

## Approach and alternatives

Keep the existing Ratatui renderer and telemetry, put profile selection and service actions in a Dashboard overlay, and use live engine identity and bounded journal snapshots.

- Keep two screens with a small modal for the occasional action. Retire the TUI-specific sweep, editors, document reader and legacy auxiliary service polling. Leave the standalone CLI, research notebooks, and Officina coding TUI available.

## Scope and documentation

Change only the Rust operational TUI, its launch/help docs, and the session anchor. Baseline: seven tabs, legacy user-unit controls, file-only logs, alias-only model identity and a gauge panel that inherits terminal colors in unpainted cells. No production service restart or profile mutation is needed for this change.

## Invariants

1. Dashboard still reports decoding, slot activity and GPU metrics. Model identity, slot fill and engine GPU come from live data. Verify with server-schema fixtures and a controlled terminal render.
2. Profile selection writes through the CLI and remains separate from system service lifecycle. Verify exact command arguments with fakes and expose action failure without claiming a restart.
3. Logs stay bounded, support freeze and search, and keep engine output available if journal access fails. Verify source keys, Unicode handling and journal error behavior.
4. Panels and gauge tracks explicitly paint theme colors. Verify the Aether frame buffer including empty gauge cells and the page background.

## Verification

- Run Rust tests and formatting, inspect Aether output and state transitions, rebuild the binary used by vitriol tui, and verify service commands without restarting production.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T13:16:28+01:00 — Plan created before implementation.

- 2026-11-26T13:30:44+01:00 — Built the release binary used by vitriol tui and the installed symlink. Fifty focused Rust tests passed and the pseudo-terminal run used isolated health, model, slots, GPU, journal and systemctl fixtures. Profile save and denied system restart were exercised separately. The terminal emitted only Aether ink and panel backgrounds for the dashboard, including both parts of the gauge. Dashboard, logs and picker captures are kept under /tmp/vitriol-two-tabs/. The native Praetor comparison recorded 44 blocking diagnostics with --warn versus 70 on the baseline. Remaining issues include complexity warnings in the touched and legacy code. Final scoped evidence goes under /tmp/vitriol-two-tabs/verification/.

- 2026-11-26T13:31:42+01:00 — Final Clippy exposed an iterator warning in the TUI's Intel fallback and two eager-value suggestions in retained draft-counter tests. Apply the equivalent local fixes and rebuild and recheck all scoped inputs.

- 2026-11-26T13:32:12+01:00 — The three retained source/test lint sites were corrected. Cargo Clippy completed without diagnostics. The scoped gates will verify the committed tree and retain the separate Praetor comparison.
