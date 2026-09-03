# Officina TUI Polish — 2026-09-02

Owner requests (this session), target = Ratatui TUI (`officina/tui/`, ported
2026-09-02) + shared `session-panel` extension. JS UI untouched (deprecation
path). Owner approved all decisions including 🜎 for AI.

## 1. Watermark — glimmer + precise centering

- `watermark.rs`: `GlimmerMode { Shimmer, Breathe, Twinkle, Off }`;
  `render(frame, area, phase_ms, mode)`.
- Vertical centering: `y = area.y + (area.height - logo_h) / 2` inside the
  watermark area (chat minus 4 editor rows) — was bottom-anchored
  (watermark.rs:40). Horizontal block-centering unchanged (left_pad from max
  line width).
- Shimmer: diagonal band `(x + y) - phase`, ~6 cells wide, peak `#4a5f7d`
  over base `WATERMARK #2a3a52`, ~8 s/sweep, via `theme::lerp_color`.
- Breathe: whole-stone lerp `#2a3a52 ↔ #35485f`, ~5 s sine cycle.
- Twinkle: deterministic hash(cell, step) — sparse cells brighten briefly;
  no RNG state.
- `tui/mod.rs`: dynamic tick — 90 ms when `entries.is_empty()` (glimmer
  animates), else 500 ms heartbeat. Phase from `started: Instant` in AppState.
- Setting (owner: "Christmas lights" configurability): `/glimmer` local
  command cycles Shimmer → Breathe → Twinkle → Off; `LOCAL_COMMANDS` +
  `local_dispatch`; persists to `~/.vitriol/officina/tui-glimmer`;
  `OFFICINA_GLIMMER` env override.

## 2. Header — mode chip replaces "officina"

- `layout.rs` header: `🜖 officina` → `☉ BUILD · tab` (glyph + bold label in
  mode color, `· tab` muted hint). Fallback `🜖 officina` until the
  agent-mode widget arrives. Model id / streaming indicator / session label
  unchanged.
- `state.rs` `parse_agent_mode`: also capture CURRENT label (token after
  glyph; loud format `► PLAN MODE — …` handled) → `agent_mode_current` +
  `agent_mode_glyph`. Existing `agent_mode` (NEXT, drives TAB) unchanged.
- `theme.rs`: `SILVER: Rgb(0xC0,0xC7,0xCF)`. Rust-side map: PLAN → ☽ silver,
  BUILD → ☉ gold, custom → widget's own glyph + gold.

## 3. Sidebar

- Title `" OFFICINA "` (gold); drop `.title_bottom(" officina ")`.
- Filter `agent-mode` widget from sidebar body (mode lives in header now —
  kills the verbose top-right text).

## 4. Composer

- Drop `" officina "` border title; plain rounded panel.

## 5. Chat labels → alchemical (owner: 🜎 for AI confirmed)

- User `☿` MERCURY (U+263F, wide font support) in green.
- AI `🜎` PHILOSOPHERS SULFUR (U+1F70E) in gold. No dedicated Unicode
  Philosopher's Stone glyph exists (verified Unicode 16 name scan);
  🜎 is the literal-name match. Fallback one-const swap: 🜀 Quintessence
  (theme.rs `GLYPH_QUINTESSENCE`, reserved-unused).
- Streaming line: `🜎` gold (drops 🜂 fire reuse).

## 6. Dividers (session-panel extension; JS-safe)

- `CONTENT_W = SIDEBAR_W - 2 = 40` — Rust panel borders eat 2 columns; 42-wide
  lines wrapped 2 chars onto the next row (owner bug report). All
  `truncate(.., SIDEBAR_W)` → `CONTENT_W`.
- `thickDiv` = `─`×40 (regions: div1); `thinDiv` = `-`×40 (subregions:
  div2/3/4) — retires `. . .`.

## Verification

- `cargo test` + `cargo build` in `officina/tui`
- `npx vitest --run .pi/extensions` + typecheck in `officina/`
- No `build-patch.mjs` changes (generated file untouched) — canaries
  unaffected.

## Implemented (2026-09-02, this session)

All of the above, plus the mid-implementation addition:

- **Motto below the stone** (owner, "aesthetic silly goose"):
  `assets/ascii-motto.txt` embedded via include_str! and rendered one blank
  row below the logo as part of the same block — shared two-axis centering
  and glimmer coordinate space (shimmer sweeps through it, twinkle can
  glint it). Included only when `area.height >= logo + 1 + motto_rows`;
  otherwise stone alone, silently.
- Unit tests added: agent-mode widget parse (quiet + loud formats),
  glimmer cycle/set. 20 cargo tests + 506 vitest tests pass; release build
  clean (36 pre-existing warnings, none in touched files).

Files touched: `officina/tui/src/{theme.rs,watermark.rs,tui/{state.rs,layout.rs,mod.rs}}`,
`officina/.pi/extensions/session-panel/index.ts` (divider width/style +
CONTENT_W truncation).

## Addendum — surface reshuffle + integrated art (2026-09-02, same session)

Owner follow-ups after seeing the first build, all implemented + shipped to
`~/.local/bin/officina` (21:01 build, 20 cargo tests green):

1. **Brand restored top-left**: header reads `🜖VITRIOL·OFFICINA` (gold bold,
   literal `·`). The mode chip moved OFF the header.
2. **Mode chip on the composer**: the prompt-box border title now carries
   `☉ BUILD · tab` / `☽ PLAN · tab` (bold symbol+label in mode color, muted
   hint, `bg(PANEL)` so it sits on the border). Titleless until the
   agent-mode widget reports in. `mode_spans()` gains a bg parameter;
   `composer_title()` helper added.
3. **Sidebar titleless**: ` 🜖 OFFICINA ` removed — the panel is a plain
   dim-bordered rounded block (brand in header, mode in composer).
4. **Integrated watermark art**: owner re-saved
   `assets/braille-logo-80c-motto.txt` (38 rows: stone + motto) — that file
   renders whenever `height >= 38`; the plain 32-row stone covers 32–37;
   below that, silence (unchanged). The separate ascii-motto concatenation
   machinery is deleted; glimmer flows through the whole block as one
   coordinate space (shimmer sweeps the motto rows too).
5. **Polish batch**: shimmer period 8s → 3.5s; header brand gains a space
   (`🜖 VITRIOL·OFFICINA`); coupling line drops the model suffix (P11 row
   below shows it); agent-mode setMode no longer notifies (footer noise);
   composer hint reads `· tab to switch mode`; unnamed-session header reads
   `SESSION ID: #01a06380`.
6. **Composer flames (the big one, owner: "as inference draws more power
   from the GPU it gets more intense and decolors into alchemical
   colors")**:
   - Signal: `_shared/engine.ts` spawns `nvidia-smi --query-gpu=power.draw,
     power.limit,utilization.gpu` per 700ms poll (fire-and-forget, ENOENT
     latches off, never throws). Load = max across GPUs of
     `gpuFireLoad()` (decode.ts): power fraction past a 25% idle baseline,
     util% secondary at 0.95×, 0.06 dead zone so desktop-idle noise stays
     dark (this host idles at 3–4% of cap — verified live). Activity proxy
     (`busy/ingest/tps`) when nvidia-smi is absent.
   - Transport: vitriol-decode emits widget `engine-fire` → `FIRE 0.731`
     (raw, machine-readable; `FIRE 0.000` when engine down → honest fade).
     OFFICINA_FIRE=0 stops emission. Rust parses + filters from sidebar.
   - Render: `tui/fire.rs` — flame strip OVERLAYING the chat's bottom rows
     (owner decision: burn over, stay readable — density ceiling ⣷, never
     ⣿), anchored to the composer's top edge. Height grows with load
     (2→6 rows), per-column envelope + hash flicker (~8Hz), density ramp
     ⠁→⣷, ALCHEMY color arc SILVER→GOLD→ORANGE→RED (nigredo→rubedo),
     hot-at-base gradient, whole-fire heat scaling with load.
   - Dynamics: exponential low-pass (τ≈0.33s, dt-based) in `step_fire`;
     90ms tick whenever `fire_level > 0`; `/fire` toggles (on|off|bare),
     persists `~/.vitriol/officina/tui-fire`, OFFICINA_FIRE=0 kill switch.
   - Tests: 2 cargo (parse+low-pass, toggle+kill), 9 vitest
     (gpuFireLoad + fireLoad incl. dead zone on this host's real idle
     reading 5.31W/170W). 22 cargo + 515 vitest green; bin installed 21:47.
   - Owner refinement (same session, "instead of flaring up over the
     middle"): envelope inverted — dead middle band (±12% of span), full
     burn at the edges with a soft shoulder (pow 0.65), 1.15× edge reach
     so a full load runs the whole strip height along the sides. Plus
     rising embers: side columns (edge > 0.3) pop a ⠁ spark above their
     flame tip — silver/gold, drifts up over 4 flicker slots (~0.5 s),
     then the hash rerolls it (EMBER_ODDS 0.09 per column per window). Bin
     installed 22:00.
   - Configurable voices (same session, "have the default be this emerald
     green fire"): `FireStyle { Emerald, Alchemy }` in fire.rs — Emerald
     (default) = deep emerald → Vitriolum green → solvent mint (the
     living fire); Alchemy = the original silver→gold→orange→red arc.
     `/fire <style>|style` switches/cycles (naming a style ignites),
     persisted in `~/.vitriol/officina/tui-fire` as "<on|off> <style>"
     (bare legacy lines stay valid), `OFFICINA_FIRE_STYLE` env override.
     23 cargo tests green; bin installed 22:07.
7. **Light yellow status voice + Pulse fire (same session)**: header
   `🜂 working`, chat `🜎 working…`, and `⚗ compacting context…` moved to
   `theme::info()` (#FFE066 — the same informational tint the engine TUI
   uses for "prompt-eval …"); orange retired to genuine warnings only.
   Third fire voice `FireStyle::Pulse`: full alchemical palette
   [silver, green, gold, cyan, violet, orange, red] cycling over ~9 s with
   wraparound lerp (`cycle_color`), hue offset by heat and row fraction so
   color waves travel up each column; shape/density/embers unchanged.
   Cycle order emerald → alchemy → pulse. 23 cargo tests green; bin
   installed 22:23.
8. **Bugfixes (same session)**: shimmer sweep now LOOPS (pos ran past the
   stone once and stayed there — sweep is 45% of each 3.5 s cycle, then
   stillness while the phase wraps, `SHIMMER_SWEEP_FRAC`); footer's orange
   "stack unreachable — no agent telemetry" demoted to a muted gray
   " ·  no telemetry" hint (bare projects have no officina extensions —
   absence of telemetry is not a fault; owner hit it in Projects/ontic).
   Working/compacting status moved to light yellow `theme::info()`.
   23 cargo tests green; bin installed 22:37.
9. **Officina reachable abroad (owner: "shouldn't officina be reachable
   from any project folder, so long as VITRIOL is running?")**: the bridge
   now carries `agent-mode` + `vitriol-decode` into any session dir
   without its own `.pi/extensions` (pi `-e <dir>`, directory form
   verified live: `mode` command registers from a foreign cwd). Root
   derived from the pi binary path (`node_modules/.bin/pi` →
   `<officina>/.pi/extensions`) so a relocated checkout keeps working;
   nothing carried when the project does its own discovery (no
   double-load). Deliberately NOT carried: knowledge-inject / task-state /
   session-panel et al. — VITRIOL-workflow context would poison foreign
   projects, and `setSidebar` does not exist on pi's RPC surface (grep
   verified), so session-panel has no TUI effect outside home. Foreign
   sessions get: mode chip + TAB cycling, composer fire, decode gauge,
   model/command registry. 25 cargo tests green (2 new carry tests); bin
   installed 22:42.
10. **Final polish batch (same session)**: shimmer period 3.5 s → 6 s;
    prismatic (pulse) is now the DEFAULT fire voice — renamed from "pulse"
    to "prismatic" (owner's word), "pulse" kept as parse alias; carry list
    gains session-panel so foreign projects get the full sidebar (its
    setWidget RPC fallback already works — verified live: full registry
    arrives over RPC); session-panel sections expanded to show scratchpad
    CONTENT (2 facts ▪ + 2 leads →, not just counts) and task ITEMS
    (open-first, ≤4 lines, [>]/[ ] marks) — owner: "I'd like to see the
    scratchpad and todo in the sidebar"; ALL dividers now the thick rule
    (thinDiv = thickDiv, hyphen sub-lines retired). 25 cargo + 515 vitest
    green; bin installed 22:53.
11. **`.pi` naming question → BRANDING SHIM (owner: "make the shim for
    unambiguous branding")**: two namespaces, two owners — `.pi/` stays
    the engine's (vendor discovery, never renamed); `.officina/` becomes
    the canonical home of OUR per-project artifacts: scratchpad already
    wrote there (`.officina/SCRATCHPAD.md` — owner's live file, which
    prompted the ask); task-state rebranded `.pi/tasks` →
    `.officina/tasks` (reads fall back to legacy, writes canonical),
    plan-mode `.pi/approved-plan.md` → `.officina/approved-plan.md`
    (implement reads fall back to legacy), background-lane cards →
    `.officina/background/<stem>/` (write-only). Bridge carry skips
    projects with `.officina/extensions` too (branded = self-curating).
    A stray dangling `officina/.officina/.pi` link (my first ln -sfn
    into the pre-existing dir) removed. Docs: OFFICINA.md "Project
    branding" section. 25 cargo + 515 vitest green (1 stale test
    assertion updated); bin installed 23:00.
12. **Abroad provider fix (owner hit it live in ontic: "No API key found
    for the selected model")**: the `llamacpp` provider is registered by
    the llama-cpp-provider extension — a project extension, so foreign
    sessions fell back to pi's stock google provider with no key, model
    "unknown". llama-cpp-provider added to the carry list (self-contained
    imports; models.json resolves via import.meta.url → cwd-independent).
    Verified live from Projects/ontic: get_state now resolves
    `provider: llamacpp, Qwen3.8-9B-Q8_0.gguf @ 127.0.0.1:8279`. When the
    engine is actually down abroad, the sidebar says so honestly — the
    owner's stated precondition ("so long as VITRIOL is running") holds.
    Also inspected ontic for the owner: cargo check is CLEAN (0 errors vs
    the scratchpad's 30-error baseline; 1 dead-code warning) — the two
    "open" tasks (main.rs callers, lint.rs consumers) describe work that
    already compiles; the abroad agent had died on the key error before
    ticking them. 25 cargo tests green; bin installed 23:10.
13. **Scrollback (owner: "make sure I can use pgup and pgdn and also
    scrolling")**: the chat renderer clamps `state.scroll` (rows back
    from the live tail) against the true line count and slices the view
    from `total - visible - scroll` — offset-from-bottom anchoring, so
    streaming output keeps flowing beneath a scrolled-back view. PgUp/PgDn
    page ±20 rows; the input task now forwards ALL crossterm events (was
    key-only) and mouse capture is enabled — wheel = ±3 rows. Home/End
    are dual-mode: empty input jumps to the oldest / live tail; while
    typing they still move the cursor. Sending a message returns the view
    to the tail. Scrolled state shows a muted `↑ n/max` badge top-right
    of the chat column. Trade-off: with mouse captured, text selection
    needs Shift+drag in most terminals. 25 cargo tests green; bin
    installed 23:41.
14. **Text priority over fire (owner, after live testing)**: fire render
    moved BEFORE the chat paragraph — the transcript paints on top, flames
    show only through blank regions as a backdrop. The scroll badge keeps
    its own top placement (it's text too). 25 cargo tests green; bin
    installed 23:44.
15. **Fire tint on ALL text (owner: "user text discolors based on the
    fire beneath it — could you do the same for AI text?")**: the user-text
    effect was ratatui style-patch semantics — unstyled spans inherit the
    flame fg; markdown-styled AI spans overrode it. `fire::render` now
    returns a per-cell color map of the strip, and the chat renderer walks
    it post-paint, forcing the flame color onto every non-space text glyph
    inside the strip — AI, tool, and diag lines all catch fire color when
    standing in flames. 25 cargo tests green; bin installed 23:51.
