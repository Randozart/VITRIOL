# Theme, Fire & Mode Overhaul — Plan (2026-11-19, PLAN mode)

## Findings (from code)

### 1. `/` menu background leak
`render_command_popup` (layout.rs:1625) DOES `frame.render_widget(Clear, area)` and
sets `.bg(theme::PANEL())` on block, detail, and rows. The leak path: `Clear` only
paints cells inside `area`, but spans rendered with an explicit `.bg(...)` of
`theme::BG()` (chat text beneath) are cleared — the likely actual defect is the
popup's computed `popup_area` at layout.rs:1050 not covering the full logical
menu rect (e.g. detail-wrap overflow or the area being sized from `cands.len()`
while the description wraps taller), OR rows whose desc is empty render spans
without an explicit bg (Span::raw / Span::styled without .bg) — raw spans keep
the *cleared* cell bg (Reset → terminal default), which under a transparent-
bg terminal shows the chat text that was under the popup is gone but gaps get
terminal-default bg, not PANEL. Fix: give the block a full-area bg fill
(paragraph of spaces) in addition to Clear, and verify popup_area height math.
Contrast note: owner says "background text leaks through" — need a repro step
first (set a busy transcript, open `/`, screenshot).

### 2. Theme architecture is already data-driven (good)
- `Theme` struct + 3 statics (`VITRIOL`, `AETHER`, `ALKAHEST`) in theme.rs;
  `ALL` array; `by_name` with alias table; `/theme` command already lists from
  `ALL`.
- palette.json (officina/theme/palette.json) is the single source of truth and
  a test (`palettes_must_equal_palette_json`, theme.rs:792) asserts statics ==
  JSON. Adding a theme = add static + JSON entry + ALL + aliases.
- Contrast gate already exists: `every_theme_is_readable` (7:1 body, 3.5:1
  secondary) — extend it with a logo-contrast assertion.
- Watermark/logo colour = `theme::WATERMARK()` (watermark.rs:230,365,385).
  Aether's leak: `watermark: 0x2A2A37` vs `bg: 0x1F1F28` — contrast ~1.1:1,
  near-invisible. That's the owner's "aethership logo insufficient contrast".
- **Rename aether → aethership**: `name: "aether"` field, ALL entry, by_name
  aliases ("kanagawa"/"aetherskiff"/"kanagawa-wave"/"aether"/"aethership"),
  state.rs:4771 help text, state.rs:9143 tests, palette.json key, and the
  theme-file resolution in `~/.vitriol/officina/tui-theme` (old stored value
  "aether" must still resolve — keep it as an alias, do not orphan it).
  Note the module docstring already says "aetherskiff" — rename completes
  that intent.

### 3. Fire: "bottom two rows same color" + inward gradient + glow
Current mapping (fire.rs render loop):
- `heat = base_heat * (1 - frac*0.75)`, `frac` runs base→tip, so colour IS
  already vertical (hot bottom → cool tip). The rule row (layout.rs:806)
  uses `ignition_color` = `style.color(base_heat)` — same heat value as the
  body's bottom row (`frac≈0` gives `base_heat*1.0`), hence **rule and first
  body row are (near-)identical colour** — the owner's observation. Past edits
  (2026-11-16/17) synced the rule to the field deliberately; colour sync is
  the leftover.
- Owner wants: colour distributed **inward→outward** (horizontal: hot core
  centre, cool edges) instead of purely bottom→top. The field already computes
  `edge = column_edge(x, w)` (0 centre → 1 edge) and `column_light` — a
  horizontal heat term is a cheap addition:
  `heat_h = 1.0 - edge_norm` (or `1 - |edge - TONGUE_CENTER|/half`), combined
  as `heat = base_heat * mix(vertical_term, horizontal_term, 0.5)` or
  `heat = base_heat * v * (0.55 + 0.45*h)` — pick so the rule row stays
  distinct (rule keeps a slight +Δheat and its own edge falloff, which it
  already has via `edge_falloff` in line_cells).
- Apply to ALL THREE styles uniformly (Pulse/Emerald/Alchemy share the `heat`
  → `color(t)` mapping, so one change covers all; per-style stops get
  "prettier glow" via more stops + brighter hot end).
- **Glow** (no native glow in ratatui): achieved by gradient density — add 1-2
  mid stops to each arc, raise the hot-end toward near-white via
  `lerp(color, WHITE, 0.25)` at the hottest cells, and let the tint mask
  borrow the flame colour wider (already shape-based). Optionally a dim
  "halo" pass: cells just outside the envelope at very low alpha colour —
  braille dots at DENSITY[0..1] in `lerp(BG, flame_color, 0.25)`.

### 4. Size ratchet (CONSTRAINT)
layout.rs 5,617 / fire.rs 1,048 / theme.rs unbudgeted (<1000, currently 945).
New code must go in NEW modules or budgets raised in-commit with justification.
Plan: fire gradient/glow logic → stays in fire.rs (raise to ~1,100, justified);
new themes are data rows (theme.rs stays <1000 by keeping statics terse — else
raise); popup fix is small (layout.rs +~15, raise budget note).

### 5. New themes (owner list)
Each = one `Theme` static + palette.json entry + ALL slot + aliases:
- **Monokai** — bg 0x272822, classic Monokai palette (pink/green/orange/
  purple), syntax "monokai" (syntect has it built-in).
- **Catppuccin Mocha** — bg 0x1E1E2E, mauve/lavender accents, syntax
  "catppuccin-mocha" (check syntect availability; fallback base16).
- **Mortal Kombat ("kombat")** — original: dragon-obsidian. bg near-black
  cold 0x0A0A0C, gold/amber dragon logo, blood red accent, ice-blue
  secondary (Sub-Zero vs Scorpion), fire arc = gold→orange→blood-red.
- **aethership** (renamed) + fix watermark contrast: raise `watermark` from
  sumiInk2 to boatYellow2-adjacent dim gold (e.g. 0x49495f → better: dim carp
  0x545a6e is still low; target ≥2.5:1 vs bg per logo gate below).
- Also re-check vitriol/alkahest watermark contrast (0x2A3A52 / 0x1E2A1E vs
  bg — both ~1.3:1, the logo is DIM by design (watermark art) — decision:
  logo gate = ≥1.8:1 "faint but present", not body-text 7:1; owner confirms).

### 6. Theme-aware fire stops already partially true
Emerald/Alchemy derive from palette accessors; Pulse too. New themes
automatically get correct arcs. Verify kombat's fire ramp in its Theme.fire.

## Plan (BUILD-mode stages)

**S1 — `/` menu bg fix + repro test**
- Diagnose exact leak: raw spans vs popup_area math (read layout.rs:1020-1060
  + render loop). Fix = explicit bg on EVERY span + full-surface fill before
  rows; assert via existing `check_menu_sizes` pattern + new unit test that
  every rendered span in the popup carries `.bg == PANEL`.

**S2 — rename aether → aethership**
- theme.rs name+aliases (keep "aether" resolving), palette.json key rename
  (test asserts equality — update together), state.rs help/tests, ALL entry.
- Stored-preference compat: tui-theme file with "aether" still resolves.

**S3 — new themes**
- Add Monokai, Catppuccin (Mocha), Kombat statics + JSON + ALL + aliases;
  extend `every_theme_is_readable` data loop (it iterates ALL — automatic).
- Verify syntect theme names exist in markdown.rs's resolve path; pick
  nearest built-in otherwise.

**S4 — logo contrast gate + fixes**
- Add `logo_watermark_contrast` test: `contrast(t.watermark, t.bg) >= 1.8`
  for every theme (faint-by-design threshold; owner confirms value).
- Fix aethership (and any failing theme) watermark values.

**S5 — fire inward→outward gradient + glow, all styles**
- fire.rs: combine horizontal edge term into `heat`; rule row keeps +Δ so
  rule ≠ first body row (fixes "bottom two rows same color" properly).
- Add glow: extra mid stops per style + hot-end white-lerp + optional halo
  embers outside envelope. All three styles (Pulse/Emerald/Alchemy) get the
  convention + prettier arcs.
- Update fire.rs tests (they assert heat mapping — will need the new expected
  shape; negative control: horizontal-uniformity no longer asserted).
- Raise fire.rs ratchet budget in-commit.

**S6 — gates + ship**
- cargo test (theme suite auto-covers new themes), fmt, clippy.
- Rebuild + reinstall TUI (operator runs ~/.local/bin/officina) — MANDATORY
  per AGENTS.md.
- Manual visual pass by owner per theme: logo visible, menu opaque, fire glow.

---

# ADDENDUM (owner request, same session): New agent modes + TAB mode menu

## Findings (from code)

### Mode system is already a registry — new modes are DATA
`_shared/agent-mode.ts`: `ModeDef` {name,label,glyph,color,hint,directive,
enterDirective,blockNonMdWrites}; `DEFAULT_MODES` = build/grind/steer/inquire/
plan; `~/.vitriol/officina/modes.json` (or OFFICINA_MODES) merges over
defaults WITHOUT code changes. TAB cycles via `nextMode()`; the TUI only
*parses* the badge (`parse_agent_mode`, state.rs:3553) — mode switching lives
in the extension. So: RESEARCH / PIPELINE / CONVERSATION are first new
`DEFAULT_MODES` entries (plus tests), no new extension needed.

### A deep-research engine ALREADY EXISTS
`officina/.pi/extensions/deep-research/` — Scope→Research pipeline (phases
1–4) with read+browse sub-coder fan-out (webfetch/websearch/browser, NO bash),
budget/judge/metrics, watchdog timeouts, and a WRITE phase handed to the main
agent. The RESEARCH mode's directive should instruct the agent to drive this
pipeline (or its orchestrate()) rather than ad-hoc browsing — reuse, not
rebuild.

### Pipeline (Research+Plan+Grind) composition
Existing pieces: deep-research (research), PLAN directive + blockNonMdWrites
(plan+confirm), grind rules.ts loop (continuation until GRIND: DONE,
caps OFFICINA_GRIND_MAX_TURNS/MINUTES, user-stand-down), compact-on-finish
(post-loop compaction, already wired). PIPELINE = a mode whose directive
chains them with explicit user gates:
  1. RESEARCH phase (deep-research pipeline, online, best-practice)
  2. present findings → ASK the owner to confirm/correct the plan (hard gate
     — the pipeline STOPS; this is owner's "asking if the plan is accurate")
  3. on confirmation: write the plan *.md with invariants + test definitions
  4. GRIND: implement to task-list-zero with the defined tests as gates
  5. compact-on-finish runs as it already does
Phase tracking via update_tasks; the mode directive names the gates so the
model cannot skip the confirm step.

### CONVERSATION mode
INQUIRE minus the repo-work framing: discuss the repo, lightweight research,
opinions/architecture chat. blockNonMdWrites false (it may browse/read),
directive says "no state changes, no task lists". Distinct colour (coldBlue is
BUILD's; use mercury/silver).

### TAB mode popup menu (owner request)
Today TAB = instant cycle (mod.rs:1007 area handles Tab→page tabs; mode cycle
is the extension's `/agent` + TUI badge hint). Adding: pressing TAB on the
Vitriol tab when a mode popup is desired opens a MODE POPUP (list of all
modes from agent-mode.json state, arrow keys + Enter to select, Esc/TAB to
dismiss, typed digit for direct pick — same interaction grammar as the `/`
command popup, which already has cand_sel + arrow handling precedent at
mod.rs:1761/1926). Ownership question: modes live in the EXTENSION, but key
dispatch + popup rendering live in the RUST TUI. Options:
  (a) TUI-native popup: TUI reads ~/.vitriol/officina/agent-mode.json (the
      facts file the extension already writes) to list modes + current; Enter
      sends `/agent <name>` into the composer and submits. No protocol change.
  (b) Extension-driven: an agent-mode popup widget via the existing sidebar/
      band contracts. Slower, more plumbing.
  → Choose (a): one file read, reuses the command-popup render pattern
  (render_command_popup), zero protocol change. TAB behaviour: first TAB in
  an empty composer opens the popup; TAB with text or a second TAB cycles
  (preserves today's muscle memory). Arrow keys already handled for cand_sel.

### New modes (concrete)
Mode colour occupancy today (agent-mode.ts): BUILD coldBlue #2e5fa3, GRIND
violet #b294bb, STEER antidote (copper/orange), INQUIRE safety/green, PLAN
sovereignty/gold. All three new colours are raw `#` hex (ModeDef supports it)
and pairwise distinct from those five.

- **research** — dark blue "abyss": **#1d3a6e**. Distinct from BUILD's
  coldBlue (deeper, less saturated) and clearly visible on the vitriol bg.
  Glyph "∿" (depth sounding). Directive: investigate deeply, ONLINE via the
  deep-research pipeline, best architectural practice, citations required;
  *.md writes only (blockNonMdWrites true); ends by OFFERING the plan
  ("switch to PLAN or PIPELINE to formalise").
- **pipeline** — the philosopher's ember, **magenta #e05adf**: sits between
  GRIND violet and alchemy's rubedo red, unmistakable against all five
  existing mode colours. Glyph "⟲" (solve et coagula — the circulation).
  Directive: the 5-phase chain above with the owner-confirm hard gate.
- **conversation** — calm sea-glass teal **#2fa08a**: discussion water,
  not excavation; distinct from every mode colour and from the dark_teal
  UI role it brightens. Glyph "◇". Directive: discussion + lightweight
  research, no state changes, no task lists, concise.

### Mode colour uniqueness gate
New registry test: mode colours are unique (parse each to RGB, pairwise
distinct) and readable (contrast vs vitriol bg >= 3:1) — badges must read on
every ground, and the gate makes future collisions a test failure.

## Addendum plan stages (continue S-numbering)

**S7 — new modes (research/pipeline/conversation)**
- agent-mode.ts: three ModeDef entries + unit tests (registry round-trip,
  directive non-empty, colours resolve, TAB cycle includes them).
- modes.json docs (README/docs touch) so owner can override.
- parse_agent_mode badge tests in state.rs for the new glyphs.

**S8 — PIPELINE confirm-gate enforcement**
- The pipeline directive is prompt-level, but add a belt: grind/rules.ts
  pattern — pipeline mode refuses continuation past the research phase until
  the last assistant message answered a confirmed plan (marker like
  `PIPELINE: PLAN CONFIRMED`), mirroring GRIND: DONE grammar. Test in
  rules-style pure fn.

**S9 — TAB mode popup (TUI-native, option (a))**
- state.rs: load modes from agent-mode.json (name/label/glyph/color/hint +
  current), new `mode_popup: Option<ModePopupState>`.
- layout.rs: `render_mode_popup` modelled on render_command_popup (S1's bg
  fix benefits both).
- mod.rs: TAB dispatch (empty composer → open popup; else cycle), arrows/
  Enter/Esc handling; Enter injects `/agent <name>` + submit.
- size ratchet: state.rs/layout.rs budgets raised in-commit with justification
  (or popup logic in a NEW module `mode_popup.rs` — preferred, keeps budgets).
- Tests: popup open/close, selection wraps, Enter sends the right /agent,
  fallback when agent-mode.json missing (popup lists DEFAULT_MODES hardcoded
  fallback).

**S10 — gates + ship**
- vitest (agent-mode + pipeline rules), cargo test (popup + badge parse),
  fmt/clippy, rebuild + reinstall TUI (mandatory), owner visual pass.

## Addendum invariants
- I7: New modes are additive — existing 5 modes byte-identical; TAB cycle
  order stable (new modes appended).
- I8: PIPELINE never implements without the owner's explicit plan
  confirmation (hard gate, marker-tested).
- I9: RESEARCH/CONVERSATION never write non-.md files (tool gate reused).
- I10: TAB-with-text or second-TAB keeps today's cycle behaviour; popup only
  on empty-composer first TAB (muscle memory preserved, pinned by test).
- I11: Popup works with agent-mode.json missing/stale (graceful fallback).
- I12: modes.json user overrides still win over new DEFAULT_MODES entries.

---

## Invariants (original theme/fire section)
- I1: Existing themes' UI colours byte-identical (vitriol pinned by existing
  test); only aether's watermark changes (owner-requested contrast fix).
- I2: Stored theme preference never orphans (old names resolve via aliases).
- I3: Fire shape/density behaviour unchanged (2026-11-16/17 owner decisions
  preserved); ONLY the colour mapping gains the horizontal term.
- I4: `/` menu spans all carry PANEL bg — no cell resets to terminal default.
- I5: Every theme passes readability gate (7:1 / 3.5:1) + logo gate (≥1.8:1).
- I6: palette.json stays the single source of truth (equality test extended).
