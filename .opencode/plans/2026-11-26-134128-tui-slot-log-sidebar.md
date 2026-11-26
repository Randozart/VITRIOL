---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T13:41:28+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "368c572a4467cb82dd64f9b721e553ee61f5c4f4"
supersedes: []
superseded_by: []
related: []
---

# tui-slot-log-sidebar

## Purpose

Add a toggleable Dashboard log viewer and show actual engine logs filtered by slot.

## Approach and alternatives

Share the log viewer between Dashboard and Logs, index emitted slot IDs in the bounded engine tail, and preserve a separately labeled slot-transition view.

- A duplicate Dashboard viewer would make controls and filters drift. Reuse the existing viewer. Dock it on wide terminals and overlay it on narrow terminals to avoid squeezing the two GPU cards.

## Scope and documentation

Change only the Rust ops TUI and its operations guide. Baseline: two tabs with one Logs view, four sources, and Slots previously synthesized from polling. Keep those events as a separately labeled source.

## Invariants

1. Slot logs use engine-assigned IDs from structured prefixes. Unattributed lines remain available under Engine. Verify against real formatted prefixes, JSON logging and interleaved slots.
2. Other slots cannot evict an active slot's retained tail. File rotation resets the slot index, and partial writes wait for their newline. Verify with incremental append and rollover tests.
3. Sidebar and full Logs view share the source, slot selection, search and pause state. Verify keyboard routing and render both terminal widths.
4. Closing the sidebar restores the original dashboard area. Narrow overlays and their empty spaces retain the selected theme. Verify in Ratatui and a terminal fixture.

## Verification

- Test real engine log prefixes, independent slot retention and rotation, shared view controls, wide and narrow Aether layouts, then rebuild and launch a terminal fixture.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T13:41:28+01:00 — Plan created before implementation.

- 2026-11-26T13:47:20+01:00 — The release binary used by vitriol tui was rebuilt. Fifty-three Rust tests, formatting and Clippy passed. Color-enabled terminal fixtures at 80 and 132 columns exercised overlay/dock, two interleaved slot IDs, exclusion of untagged output, navigation to full Logs, search and close. Fixtures include the captured Kintsugi slot timing prefix. Native Praetor --warn remains red with 43 findings compared with 44 at baseline. Source-bound scoped evidence and captures live under /tmp/vitriol-slot-sidebar/.
