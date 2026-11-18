# Officina themes: VITRIOL, Aether (Kanagawa Wave), Alkahest (neon alchemy)

**Date:** 2026-11-18 09:10
**Mode:** PLAN (research only — nothing changed)
**Owner:** "turning the current colour scheme for Officina into the 'VITRIOL' theme, and then also
have another theme based on Kanagawa Aetherskiff" → **decided 2026-11-18: name it `aether`, and the
gold role becomes Kanagawa carp yellow `#E6C384`** (option (a) from §5, so the theme looks like
itself; the alchemical glyphs carry the identity).

**Repo state confirmed first:** `HEAD == origin/main == aa81a54`, working tree clean, CI green. The
only local divergence is the un-pushed `origin/main` ref update, which `git fetch --dry-run` applies.

## 1. The theme you made (found)

```
/home/randozart/Desktop/Projects/themes/kanagawa-aetherskiff/
├── vscode/randozart.kanagawa-wave/themes/kanagawa-wave-color-theme.json   ← the palette source
├── plasma/org.kde.kanagawa-aetherskiff/            (Plasma 6 global theme)
├── color-schemes/KanagawaWave.colors   kvantum/   gtk/   terminal/ (kitty, zellij, starship, fish, btop, broot, alacritty)
├── wallpapers/  install.sh  kanagawa-apply.sh  README.md
```
There is also `themes/alka-officina/` (a second, older theme of yours) — not used here, but worth
knowing it exists if you ever want a third.

Authoritative values for the plan (straight out of `kanagawa-wave-color-theme.json`):

| role | Aetherskiff | today (VITRIOL) |
|---|---|---|
| bg | `#1F1F28` sumi ink | `#0D1117` |
| darker bg (panels/status/sidebar) | `#16161D` | `#161B22` |
| raised surface | `#2A2A37` | — (USER_BG `#131A22`) |
| border / dim | `#2A2A37`, `#363646` | `#21262D` |
| text | `#DCD7BA` fuji white | `#E0E0E0` |
| muted | `#727169` | `#8B949E` |
| accent 1 (primary) | `#7E9CD8` crystal blue | `#FFD700` gold |
| accent 2 | `#957FB8` wave violet | `#3FB950` green |
| success / string | `#98BB6C` spring green | `#3FB950` |
| warning | `#FFA066` / `#E6C384` carp | `#FFA000` |
| error | `#E82424` / `#FF5D62` | `#F85149` |
| info / streaming | `#7FB4CA` spring blue | `#00FFFF` |
| special (number/label) | `#D27E99` sakura | `#B294BB` |
| watermark | `#2A2A37` at ~20 % | `#2A3A52` |

Kanagawa Wave is 20–30 % *lighter and lower-contrast* than the current GitHub-dark base, which
matters: the fire, the tint mask and `MUTED` text all assume a very dark ground.

## 2. What a theme has to cover (not just 19 colours)

`officina/tui/src/theme.rs` holds **19 colour constants + 9 glyph constants**, referenced
**702 times** across the crate (`PANEL` 216, `MUTED` 99, `GOLD` 97, `BG` 51, `TEXT` 43, `GREEN` 34,
`ORANGE` 30, `RED` 21, …). Three more things are colour-bearing and currently hard-coded:

1. `Style::color(heat, t)` (`theme.rs:173`) — the **fire/tint heat ramp**, which is not a palette
   entry but a gradient; a theme must own its ramp or Aetherskiff will glow GitHub-orange.
2. `markdown::highlight_theme()` (`markdown.rs:36`) — **syntect** fenced-code highlighting is a
   separate `tmTheme`, so fenced blocks would stay on the old theme unless the theme supplies one.
3. The **heat/capacity/activity ramps** used by gauges (`LIGHT_YELLOW`→`DEEP_RED`,
   `DARK_TEAL`, `MERCURY`, `COLD_BLUE`) — three ramps, each with a low/high pair.

## 3. Design (least churn, no half-themed UI)

**A `Theme` struct + one global, with accessor functions.**

```rust
pub struct Theme {
    pub name: &'static str,
    pub bg: Color, pub panel: Color, pub user_bg: Color, pub border_dim: Color,
    pub text: Color, pub muted: Color,
    pub green: Color, pub gold: Color, pub cyan: Color, pub orange: Color,
    pub red: Color, pub violet: Color, pub silver: Color, pub watermark: Color,
    pub cold_blue: Color, pub light_yellow: Color,
    /* ramps */ pub heat: HeatRamp, pub capacity: (Color, Color), pub activity: (Color, Color), pub pulse: Color,
    /* syntect */ pub syntax: &'static str,   // tmTheme name or embedded theme
}
pub fn current() -> &'static Theme;          // OnceLock, set once at startup
pub fn set(name: &str) -> bool;              // /theme switch at runtime
// accessors keep call sites short:
pub fn bg() -> Color;  pub fn panel() -> Color;  /* … */
```

Call sites change from `theme::PANEL` to `theme::panel()` — **702 mechanical replacements**,
scriptable and verifiable (`grep -c "theme::[A-Z]"` must reach 0). The alternative (threading a
`&Theme` through every render fn) is more churn than this refactor and slower to compile-test.

Rejected for now: making the palette a *build-time* feature — two binaries to install, and the
operator cannot A/B a theme without restarting the build.

## 4. Interface

- `/theme` → list + current; `/theme vitriol`, `/theme aetherskiff` (maybe `/theme aether` as an
  alias), persisted to `~/.vitriol/officina/tui-theme` (0600 not needed; it is not a secret).
- `OFFICINA_TUI_THEME=<name>` wins for a throwaway run (fingerprint-style provenance: the name is
  printed in the header on `--verbose`, or in `/theme` output).
- Startup: read env → file → default `vitriol`.
- Glyphs stay theme-independent (they are alchemical identity, not colour), but the struct leaves
  room to override `glyph_ai` etc. later.

## 5. Aetherskiff specifics (the parts that need judgement, not mapping)

1. **Contrast.** `MUTED #727169` on `BG #1F1F28` is ≈ 3.9:1, below the 4.5:1 the current
   `#8B949E` achieves. Body text is fine (`#DCD7BA` ≈ 10.9:1). Plan: use Kanagawa's own *oldWhite*
   `#C8C093` for MUTED-adjacent labels and keep `#727169` for genuinely de-emphasised text
   (comments, timestamps) — i.e. the role split gets one more entry (`faint`).
2. **Fire ramp.** Kanagawa heat: `#2A2A37` (cold) → `#957FB8` → `#D27E99` → `#FF9E3B` → `#E6C384`.
   Must keep the same *number of stops* as the VITRIOL ramp so `Style::color(heat, t)` stays a pure
   function of `heat`, and the fire tests (`flames_taper_with_height`, tint mask) keep passing.
3. **Syntect.** Ship one `.tmTheme` generated from the VSCode `tokenColors` (28 entries already in
   the theme file) and embed it; `highlight_theme()` then selects by `theme.syntax`. Fallback:
   `base16-ocean.dark` for VITRIOL (as today) and a Kanagawa tmTheme for Aetherskiff.
4. **The gold identity.** VITRIOL's gold (`#FFD700`) is the "sovereignty" accent — headers, active
   accents, the crucible. Aetherskiff has no gold; its nearest is carp yellow `#E6C384`. Decide:
   (a) use carp yellow (cohesive), or (b) keep a gold accent in both themes (identity preserved).
   **DECIDED (owner, 2026-11-18): (a)** — `gold` in Aether is carp yellow `#E6C384`, and the dim
   variant is `#C0A36E` (boat yellow) for borders/dividers. The theme should look like itself; the
   alchemical glyphs carry identity, not the hue.

## 6. Tests (the invariants that keep a theme from being half-applied)

- **I1 completeness:** every `Theme` field is set in both themes (struct literal ⇒ compile-time), and
  a test asserts no field equals the *default* `bg` by accident (catches "forgot to define").
- **I2 contrast floor:** for both themes, `text`/`bg` ≥ 7:1 and `muted`/`bg` ≥ 3.5:1 (a small
  WCAG-ish check; it is what makes a light-ish Kanagawa usable in a terminal).
- **I3 no colour leaks:** `grep "theme::[A-Z_]"` returns 0 after the refactor — pinned as a test
  that reads the source tree, because the failure mode is a *silently unthemed* widget.
- **I4 differential rendering:** the same state rendered under both themes produces identical
  *layout* (line count, each line's display width) and different *colours* (`screen_bgs` helper
  already exists and asserts background colours).
- **I5 fire ramp parity:** `Style::color(0.0, ..)` and `Style::color(1.0, ..)` differ in both
  themes, and the ramp is monotone in brightness within each.
- **I6 persistence:** `/theme aetherskiff` writes the file, a restart reads it back, an unknown name
  is refused (no silent fallback to default).

## 7. Stages

1. `theme.rs`: introduce `Theme`, move today's constants into `Theme::VITRIOL` verbatim (byte-identical
   output, pinned by a golden `screen()` test), add accessors; scripted rename of the 702 sites.
2. `Theme::AETHERSKIFF` from the table above; run the differential/golden tests; check the fire.
3. Syntect tmTheme generation + `highlight_theme()` selection.
4. `/theme` command, env override, persistence, docs (`officina/docs/themes.md`).
5. Screenshot pass at 80/120/200 columns in both themes (the operator's eye is the final gate).

## 8. Open questions for the owner

1. **Gold accent in Aetherskiff:** carp yellow (cohesive) or keep gold (identity)? I lean carp.
2. **Name:** decided — `aether` (`/theme aether`). The asset directory stays `themes/kanagawa-aetherskiff`.
3. **Third theme?** deferred — `themes/alka-officina` exists with its own palette; the `Theme` struct
   makes a third a data-only addition later. Two themes for now.

## 9. Isolation: yes, a worktree (owner asked)

**Recommendation: do it in a git worktree on a branch.** Reasons, in order:

1. **The installed binary is the operator's TUI.** A 702-site mechanical rename can leave the crate
   uncompilable for a stretch; working in the main worktree means every intermediate state is one
   `cargo build --release && cp` away from replacing `~/.local/bin/officina` with something broken.
   In a worktree, `main` stays green and the installed binary stays untouched until the final,
   verified build.
2. **A/B is the whole point of a theme.** Two binaries (old and new) must be runnable one after the
   other; a worktree with its own target dir gives exactly that.
3. **Precedent and room.** This repo already runs two worktrees (`VITRIOL-lull`, `VITRIOL-vllm`), and
   `/scratch` has 108 G free — a full cargo target for the TUI is 5.2 G.

Concretely:

```bash
cd /home/randozart/Desktop/Projects/VITRIOL
git worktree add ../VITRIOL-themes -b feat/officina-themes
cd ../VITRIOL-themes/officina/tui
CARGO_TARGET_DIR=/scratch/targets/officina-themes cargo test      # NOT /tmp (AGENTS rule)
```

Rules for this worktree:

- **Rust only.** The theme change touches no TypeScript, so `npm ci` is unnecessary there — the
  extension suites keep running in the main worktree. (If a change ever needs both, run
  `npm ci` in the worktree rather than symlinking `node_modules`: the latter confuses jiti caching.)
- **One binary at a time.** Both worktrees share `~/.vitriol/officina` (session state, slot mirror,
  route ledger). Never run the worktree TUI and the installed TUI side by side — they would fight
  over the session file and the KV slot. A/B sequentially.
- **Install last.** Only after the worktree's suite is green and the visual pass is done:
  build release in the worktree, `cp` to `~/.local/bin/officina`. That single copy is the only step
  that reaches the operator's TUI.
- **Land it as one merge.** `git switch main && git merge --ff-only feat/officina-themes` (or a single
  squash commit) so `main` never contains a half-migrated palette; CI then runs on the merged tree.
- **Clean up:** `git worktree remove ../VITRIOL-themes` and `rm -rf /scratch/targets/officina-themes`
  after the merge, so the 5 G goes back.

Not needed: a separate clone (no benefit, doubles `engine/` + `node_modules`), or a stash-based
flow (no isolation at all).

## 10. Third theme: "Alkahest" (`themes/alka-officina`) — and a naming collision to resolve

**Owner:** "Also, Alkahest for alka-officina."

Reading that theme changes one assumption: **its base IS today's Officina palette.**

| role | alka-officina | Officina today |
|---|---|---|
| background | `#0D1117` (plasma 13,17,23) | `#0D1117` — identical |
| panel | `#161B22` (plasma 22,27,34) | `#161B22` — identical |
| text | `#E0E0E0` | `#E0E0E0` — identical |
| muted | `#8B949E` | `#8B949E` — identical |
| accent | `#DFFF00` swirl yellow (cursor/focus/active borders) | `#FFD700` gold |

So "VITRIOL = today" and "Alkahest = alka-officina" would be *the same theme with a different accent
hue*. That is not worth a menu entry. What actually distinguishes Alka Officina is its **neon**
identity — its own README calls it "neon alchemy… electric accents":

| role | Alkahest (proposed) | source |
|---|---|---|
| bg / panel / text / muted | unchanged (`#0D1117` / `#161B22` / `#E0E0E0` / `#8B949E`) | alka plasma |
| gold / accent | `#DFFF00` swirl yellow | alka brand |
| green | `#39FF14` neon | kitty `color10` |
| cyan / info | `#44AAFF` electric blue | kitty `color4` |
| violet | `#FF00FF` magenta | kitty `color13` |
| orange | `#FFD700` (bright) | kitty `color11` |
| red | `#FF4444` | kitty `color1` |
| rosé (if needed) | `#FF79C6` | kitty `color5` |
| silver | `#C0C7CF` (unchanged) | — |

Result: three themes that are each recognisably themselves — **vitriol** (gold on GitHub-dark, today),
**alkahest** (neon alchemy on the same base), **aether** (Kanagawa Wave).

Implementation cost is nil beyond a third `Theme` literal (the struct is data): one more match arm in
`/theme`, one more name in the persistence test. So the staging in §7 gains a step 2b: add
`Theme::ALKA_HEST` next to `AETHER` and extend the completeness/contrast tests to three palettes.

Note for the *fire* ramp: Alkahest wants its heat ramp to reach swirl-yellow rather than gold at the
top, otherwise its fire looks like VITRIOL's — the ramp is per-theme data in the struct, so this is
also a table entry, not code.

## 11. Shipped (2026-11-18, BUILD in worktree `../VITRIOL-themes`)

Branch `feat/officina-themes`, two commits (`3f16aef` + `787e8ac`), fast-forward merged into `main`.

| what | where |
|---|---|
| `Theme` struct + statics `VITRIOL` / `AETHER` / `ALKAHEST` + `current()`/`set()`/`by_name()` (aliases: kanagawa, aetherskiff, alka-officina, vitriolum) | `officina/tui/src/theme.rs` |
| 641 call sites moved from `theme::CONST` to `theme::CONST()` (mechanical, reviewable) | `src/{fire,markdown,watermark,theme.rs,tui/*}.rs` |
| per-theme syntect tmThemes | `officina/assets/syntect/{kanagawa-wave,alka-neon}.tmTheme` |
| `/theme [name]`, persistence, `OFFICINA_TUI_THEME` | `tui/mod.rs`, `tui/state.rs` |
| palette table, invariants, "add a theme" | `officina/docs/themes.md` |

Decisions taken as planned: **Aether** (not Aetherskiff) with **carp yellow** as its accent;
**Alkahest** is the neon identity of alka-officina (same GitHub-dark ground as VITRIOL, so the
accent set is what distinguishes them); `faint` role added for comment-grade text.

Tests: `theme_suite` (8) — every role defined in every theme, readability floors (body ≥7:1,
secondary ≥3.5:1), aliases/unknowns, lenient name parsing, fire ramps warm toward the hot end,
themes distinct, **VITRIOL pinned by literal** (so "it is unchanged" is a contract), and a source
scan that fails if any call site still reads a colour as a constant. Total **355 cargo tests**,
fmt clean, zero warnings.

Installed `2f9dabce`; the worktree, branch and its 5 GB target dir are removed (the branch was
fast-forwarded, so the worktree added nothing). Preview without installing anything:
`OFFICINA_TUI_THEME=aether officina` or `/theme aether` (persists).

### Known follow-ups
- The fire's `FireStyle` arcs are still operator-selectable and independent of the palette (VITRIOL's
  Alchemy arc kept byte-identical on purpose). `Theme::fire` exists for a future theme-aware flame
  and is currently consumed by the heat gauge ramp + tests.
- Alkahest's fire ramp ends in magenta (`#FF79C6`), Aether's in carp; eyeball both before declaring
  the ramps final.

### CI caught what I missed

The Vitriolum **palette parity gate** (`officina/.pi/extensions/_shared/vitriolum.test.ts`) failed on
the first push: it matched `Color::Rgb(0x..)` literals, and the refactor encodes colours through the
const-friendly `c_rgb(...)` helper, so it reported "bg #0d1117 missing from theme.rs" while the value
sat in plain sight. Fixed in `f5348ba` — the regex accepts both spellings and now scans **only the
VITRIOL block**, so a colour that exists only in Aether can no longer satisfy the parity check for
the original scheme. The gate is stricter than it was.

That is the second time a test's *representation* assumption (not its intent) hid a change: the same
shape as the fire-taper test earlier today. Worth remembering when writing source-scanning gates.
