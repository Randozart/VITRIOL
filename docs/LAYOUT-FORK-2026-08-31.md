# Layout Fork — docked sidebar, native top panel, pinned composer — 2026-08-31

**Goal:** the Crush/OpenCode shell inside Officina: chat + composer in a
main column, a **docked right sidebar** (not a floating overlay), panel
content **pinned at the top**, composer **pinned at the floor** — all
without giving up the alt-screen, watermark, or any extension behavior.

## Why a fork (recorded decision)

pi-tui's `Container`/`Box` are vertical-only; `InteractiveMode` renders a
single column: header → chat → widgets → editor → footer. Extension
overlays are the only horizontal primitive, and a *persistent* extension
overlay breaks input routing in pi 0.83.0 (PTY-proven; see
docs/OFFICINA.md bugfix log). Therefore: vendor the layout layer into
`officina/runtime/`, patch the column split + pinning there, and keep the
divergence small, headered, and upgradeable (Rule 9: conscious fork,
recorded divergence — our second, after the launcher).

## Fork surface (keep it tiny)

Only TWO files need surgery; everything else stays the pinned library:

1. `runtime/columns.ts` — NEW: `Columns` component (pure pi-tui Component):
   N children rendered side-by-side with fixed/right-weighted widths,
   ANSI-aware padding, per-frame reflow on terminal resize. Testable.
2. `runtime/interactive.js` — vendored `interactive-mode.js` with exactly
   three patches:
   - P1: wrap `chatContainer + widgets + editorContainer` in a
     `Columns(main, sidebar)` where sidebar renders the session-panel
     component; hide when terminal width < 100 cols.
   - P2: pin the editor: after layout, fill remaining viewport rows above
     the editor block (replaces the wrapper's newline reserve).
   - P3: expose `ui.setSidebar(lines)` alongside setWidget so the
     session-panel extension talks to the docked column without knowing
     about overlays.

The session-panel extension keeps its public surface (`/panel`, state,
colors) but renders through `ui.setSidebar` when the fork mode is active.

## Activation + rollback

- `OFFICINA_LAYOUT=docked` (default once parity passes) → `officina.mjs`
  imports `runtime/interactive.js`; `OFFICINA_LAYOUT=classic` → stock pi
  mode. Rollback is an env var.
- The newline-reserve composer pin stays as the classic-mode fallback.

## Parity gate (before docked becomes default)

Same task set, same fingerprint, `tris`-ledger tok/task within noise of
classic mode; PTY assertions: keystrokes echo, sidebar renders with
coupling/files/tokens, composer at floor, watermark lifecycle intact.

## Provenance

`runtime/interactive.js` carries the vendored-file header: source
`@earendil-works/pi-coding-agent 0.83.0 dist/modes/interactive/interactive-mode.js`,
Apache-2.0, patch list P1–P3 with markers `// [officina P1]` etc. The
fork re-bases by diffing against the pristine 0.83.0 file kept at
`runtime/upstream/interactive-mode.js.reference`.

## Steps

1. `columns.ts` + unit tests (ANSI widths, flex/cut semantics) - DONE
   2026-08-31: 7 tests, typecheck clean. Design note: columns pad to slot
   width and keep trailing pad (stable row widths for the TUI viewport);
   colored overflow is ANSI-safe-cut.
2. Vendor + header the reference file; wire `OFFICINA_LAYOUT` switch.
3. P3 sidebar plumbing + session-panel adapter.
4. P1/P2 layout patches; PTY suite for input + visuals.
5. Parity ledger run; flip default; delete the newline-reserve fallback
   from the wrapper (keep as classic fallback).
