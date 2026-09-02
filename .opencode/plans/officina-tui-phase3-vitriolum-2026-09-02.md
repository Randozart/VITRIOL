# officina-tui Phase 3 — Vitriolum styling, markdown, watermark, local commands, glyphs

**Date**: 2026-09-02
**Status**: EXECUTED (same day)
**Crate**: `officina/tui/` (Ratatui TUI, pi-coding-agent via JSONL RPC)

## Context

Phase 1 scaffolded the crate (RPC bridge, minimal chat, two-column layout).
Phase 2 fixed the trust root cause (`-a` flag — RPC mode silently fails
project trust → extensions + settings never loaded), stderr drain, EventStream
keyboard task, diagnostics, slash autocomplete. Live probe confirmed:
23 setWidgets / 15 commands / model llamacpp/Qwen3.8-27B / text deltas flowing.

Phase 3 ports the visual language of the VITRIOL dashboard
(`vitriol-tui/src/`, same repo, own code, Apache-2.0) into officina-tui.

## Root reference files

- `vitriol-tui/src/theme.rs` — canonical Vitriolum palette + BrailleRamp
- `vitriol-tui/src/ui.rs` — header banner, footer keybar, panel anatomy
  (`BorderType::Rounded`, PANEL bg, liveness borders, gold/dim titles)
- `vitriol-tui/src/markdown.rs` — CommonMark → styled ratatui lines (pulldown-cmark)
- `vitriol-tui/src/watermark.rs` — braille logo, tint #1c2634 + DIM, bottom-rise
- `VITRIOL/assets/braille-logo-80c.txt` — logo art (32 rows × 80 cols)

## Changes

### 1. `src/theme.rs` (new)
Verbatim Vitriolum port: BG #0d1117, PANEL #161b22, BORDER_DIM #21262d,
GREEN/safety #3fb950, GOLD #ffd700, CYAN/solvent #00ffff, ORANGE/antidote
#ff5f1f, RED/substrate #f85149, TEXT #e0e0e0, MUTED #8b949e, ramps
(LIGHT_YELLOW, DEEP_RED, DARK_TEAL, MERCURY, COLD_BLUE). Style fns:
text/muted/title/banner/live/info/warn/gold_muted/panel_border.
BrailleRamp enum (Capacity/Activity/Heat/Pulse/Power/Velocity) +
ramp_color/lerp_color/signals_danger/gauge_value_style + unit tests
(endpoints, clamping, midpoint, danger flags — ported from vitriol-tui).
Alchemical GLYPH_* constants (see §7).

### 2. `src/watermark.rs` (new)
Port of vitriol-tui watermark. One change: `include_str!("../../../assets/
braille-logo-80c.txt")` — compile-time embed (binary works from any cwd).
Rules kept: bottom-anchored, horizontally centered, no partial reveal,
TINT #1c2634 + DIM. Trigger: fresh screen = zero chat entries → watermark
rises in chat area; disappears once first entry lands.

### 3. `src/markdown.rs` (new)
Full 1:1 port (Renderer event walk, wrap_line char+style coalescing,
blockquote nesting, pipe tables with ┃ head sep, lists, rules, code blocks).
Dep: `pulldown-cmark = "0.12"` (Apache-2.0 OR MIT — compatible).
All 9 unit tests ported. Integration: Assistant entries render via
markdown::render(text, width); cache keyed (entry_index, text_len, width)
in AppState (HashMap, capped 1024) so finished entries parse once; only the
growing tail re-renders per frame. User/thinking/tool entries stay plain.

### 4. `src/layout.rs` — Vitriolum dashboard styling
- Vertical skeleton: header (1 row) · body (min) · footer (1 row)
- Header: `🜖 officina` theme::banner gold bold + model id theme::live +
  session name muted right-aligned (vitriol-tui header anatomy)
- Footer: muted keybar `enter send · tab complete · esc dissolve · ^c/^q
  quit · f9 stderr` + `🜂 working` antidote while streaming + orange
  `stack unreachable — nothing on :8279` when no widget telemetry yet
- Chat + sidebar: BorderType::Rounded, PANEL bg, BORDER_DIM borders, gold
  titles (panel_neutral equivalent); editor border BORDER_DIM idle /
  GOLD with autocomplete / DIVIDER_COLOR streaming
- All ad-hoc Color::Rgb in layout/state replaced by theme fns
- Fresh-screen watermark in chat area

### 5. Local commands (tui/mod.rs)
Enter handler intercepts BEFORE sending to pi: `/quit` → should_quit;
`/diag` → toggle stderr overlay; `/clear` → clear chat entries.
All other `/x` go to pi as prompt (extension commands, agent-session.js:985).

### 6. `src/tui/ansi.rs`, `src/tui/state.rs` (touch-ups)
state: diag entries use theme::RED, tool running theme::warn(ORANGE),
thinking MUTED+ITALIC, user GREEN/safety, assistant live()/markdown.
AppState::chat_lines signature → &mut self (markdown cache); render(frame,
&mut state) — ratatui draw closure is FnOnce, safe.

### 7. Alchemical glyph pass (verified against Unicode UCD nameslist)
Block U+1F700..1F77F. Verified names (UnicodeData.txt, latest UCD):

| glyph | cp | name | use |
|---|---|---|---|
| 🜖 | U+1F716 | VITRIOL | header banner brand mark |
| 🜀 | U+1F700 | QUINTESSENCE | (reserved) scratchpad label |
| 🝠 | U+1F76A | ALEMBIC | (reserved) compaction |
| ⚗ | U+2697 | ALEMBIC (Misc Symbols, wide font support) | compaction line |
| 🜍 | U+1F70D | SULFUR | diagnostics/errors (vitriolic) |
| 🜎 | U+1F70E | PHILOSOPHERS SULFUR | (reserved) coupling mark |
| 🝥 | U+1F765 | CRUCIBLE | tool running |
| 🜂 | U+1F702 | FIRE | working indicator (the living forge) |
| 🝡 | U+1F761 | DISSOLVE | footer abort hint ("esc dissolve") |
| 🝯 | U+1F76F | NIGHT | (reserved) engine idle |
| 🝮 | U+1F76E | HOUR | (reserved) latency displays |
| 🜔 | U+1F714 | SALT | (reserved) files |

Font-coverage caveat: U+1F700 block is sparsely covered. The four elements
(🜂🜁🜃🜄) are proven in this terminal (vitriol-tui ships them). Candidates
🜖🜍🝥 need a live render test; ASCII fallbacks (◈ ⚠ ⚙ ·) are one-line swaps
in theme.rs constants. ⚗ U+2697 assumed safe (broad coverage).
Reserved glyphs ship as constants, wired when their surfaces exist
(scratchpad/compaction/coupling/engine-idle arrive via JS widgets today).

### 8. Verification
- cargo test (markdown 9 tests + ramp tests)
- release build → ~/.local/bin/officina
- probe: fresh-screen watermark, markdown reply, /quit exits cleanly
- glyph test line printed to user terminal; non-rendering glyphs swapped
  to fallbacks

## Out of scope (later phases)
- Interactive dialogs (select/confirm/editor — auto-cancelled today)
- /history scroll modal, session tree picker
- JS-side widget glyph adoption (engine idle 🝯 etc. — extensions render
  widget lines, Rust just displays)
- Watermark in sidebar column
