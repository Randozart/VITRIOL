---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T12:50:10+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "6e8fa1ebc4b828fd793754eaf6add486d796e2af"
supersedes: []
superseded_by: []
related: []
---

# officina-input-settings-chat

## Purpose

Make autofill Right-Arrow-only, expose finite slash settings through popups, and anchor compaction status in chat

## Approach and alternatives

Reuse existing input/modal/rendering and persisted local-setting dispatch; explicit option selection for finite settings; replace floating compaction overlay with a stable chat entry/progress row; preserve extension/RPC and keyboard navigation contracts

- Changing deprecated JS frontend misses the canonical operator path. Cycling a setting on bare slash entry hides choices. A floating compaction card overlaps unrelated transcript content.

## Scope and documentation

Canonical officina/tui Rust frontend, focused input/render/event tests and operator docs. Build and install updated binary; no active engine restart or model changes.

## Verification

- Baseline/full Rust TUI tests and fmt; focused Right/Tab/cursor and selection/cancel/persistence tests; compaction lifecycle and narrow/scrollback rendering tests; Praetor delta review; rebuild/reinstall plus isolated RPC-driven terminal/render verification; frozen source-bound gates

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T12:50:10+01:00 — Plan created before implementation.

## Invariants

1. I1: Explicit existing options and unrelated commands retain their local/RPC behavior; extension dialogs retain keyboard priority. Pin with dispatch parity and full existing test suite.
2. I2: Tab never accepts slash or ghost autofill; Right accepts only at the input end, preserving Unicode cursor movement. Pin with keyboard integration tests. Enter still selects command-menu candidates.
3. I3: Bare finite-setting commands open a picker without changing values; Escape cancels; Enter applies the highlighted explicit option. Pin with selection/cancellation and dispatch tests. Existing permissions/tools/model pickers remain authoritative.
4. I4: Compaction appears once in the chat viewport, with no floating composer card; lifecycle, failures and continuation stay unchanged. Pin with render assertions and existing compaction lifecycle matrix.
5. I5: Menus remain bounded at small terminal sizes; renderer smoke matrix covers choices and narrow screens.

The UX hypothesis is falsified if a bare setting mutates immediately, selection differs from typed options, compaction covers the composer, or terminal rendering panics. Baseline: 245 Rust tests passed; original source/Praetor output saved in /tmp/officina-input-settings-evidence.

### Implementation stage — 2026-11-15 Europe/Amsterdam

Implemented Right acceptance with Tab suppressed while candidates/ghosts are visible; Tab otherwise retains agent cycling. Enter retains command candidate selection. Added local finite-setting pickers (fire including style/tint, glimmer/shimmer, sidebar, thinking, thinking-view, diagnostics), settings/config directory and existing permissions picker for yolo. Explicit fire/glimmer options reuse persistence; sidebar remains session-local as before. Existing model/tools/permissions/fx panels remain available. Arbitrary extension commands keep their own handlers. Removed duplicate compaction overlay: the existing chat progress spinner, elapsed time, finish/failure entries and continuation behavior remain authoritative.

250 Rust tests passed, including new keyboard, cancellation/selection/RPC, small-screen and compaction scrollback tests. Format and diff checks passed. Scoped Praetor currently reports 157 existing non-comment findings both before and after, with no new-setting-module findings. The short-terminal test caught and drove a correction to menu placement. Full evidence and installed-binary terminal checks will be recorded externally in /tmp/officina-input-settings-evidence; final runner directory is /tmp/officina-input-settings-evidence/final. No engine/model changes or live-session restarts.

### Verification-driven amendment — 2026-11-15T13:04 Europe/Amsterdam

The broad registry test explicitly prohibited fire/glimmer from an earlier command consolidation. The current request restores these names as local settings menus; update that affected acceptance contract while retaining duplicate/shadowing guards and other retired names. Add a positive registry assertion for the menu names. Clippy also exposed a pre-existing `.skip(0)` in the renderer; remove this no-op to allow its default deny gate to run. This is a verification repair stage, with no runtime semantics change beyond the already approved menus. The first frozen run is superseded by a fresh run after this correction. Installed-binary PTY passed menu visibility, fire/glimmer persistence, sidebar cancellation, compaction progress and clean quit using an isolated HOME/cwd. Existing scope warnings include render_main complexity increasing with picker placement (18→21 cyclomatic,17→23 cognitive), while chat-render complexity decreased (79→78,200→199); 157 non-comment diagnostic instances remain.
