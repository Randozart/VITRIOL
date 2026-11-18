# Theming the other VITRIOL surfaces

**Date:** 2026-11-18 12:40
**Mode:** PLAN (research only — nothing changed)
**Owner:** "could we do the same sort of theming for VITRIOL as well?"

Two different things can be meant, and they are very different jobs. §1 is the TUI reading
(most likely, given the last few hours); §2 is the desktop-package reading. §3 asks.

## 1. `vitriol-tui` — the ops dashboard

What it is now:

| fact | value |
|---|---|
| launched by | `vitriol tui` → `vitriol-tui/target/release/vitriol-tui` (built on first use) |
| size | 9,467 lines across 22 files (`ui.rs` alone is 2,358) |
| palette | `src/theme.rs`, **19 constants**, referenced **281 times** |
| interaction | a keyboard dashboard: a flat `match` on `KeyCode` in `main.rs`, no command bar |
| config | environment only (`VITRIOL_*`), no UI prefs section |

### 1.1 It has already drifted from Officina

```
vitriol-tui  ORANGE = #FF5F1F      ← the old, reddish orange
officina/tui ORANGE = #FFA000      ← fixed 2026-11-14: red-orange read as the failure red
```

Everything else still matches (`BG #0D1117`, `PANEL #161B22`, `GREEN #3FB950`, `GOLD #FFD700`,
`CYAN #00FFFF`, `RED #F85149`, `TEXT #E0E0E0`, `MUTED #8B949E`). So the dashboard is the last
surface still shipping the misleading warning colour — a one-line correctness fix on its own,
independent of theming.

### 1.2 The refactor is the same shape as Officina's, at 44 % of the size

`Theme` struct + `VITRIOL`/`AETHER`/`ALKAHEST` statics + accessors with the historical names
(`theme::GOLD()`), 281 mechanical call-site rewrites, then:

- `VITRIOL_TUI_THEME` env override;
- a key to cycle (e.g. `T`, next to the existing `t` for profile-select — case-distinct is fine, or
  F5 to avoid confusion);
- persistence in `~/.vitriol/tui-tui-theme` — deliberately a **separate file** from Officina's
  `~/.vitriol/officina/tui-theme`: the dashboard is a different surface and may want a different
  palette (the env var is the one lever that can set both);
- the fire/tint ramp question does not arise (the dashboard has no flames), but it does have
  gauges/braille ramps → same per-theme ramp treatment as Officina's `BrailleRamp`.

Cost estimate: roughly half a day, most of it mechanical, with the same four gates Officina now has
(no const reads, no colour literals outside the palette, contrast floors, distinctness).

### 1.3 The drift-proof alternative (and why I would not start there)

Extract `theme-core` **inside `officina/`** (`officina/theme-core/`) and have both TUIs depend on it,
so a palette can never diverge again. The constraint that shapes this: `officina/` is published as a
standalone snapshot and **nothing inside it may reference paths above itself** — so the shared crate
must live under `officina/`, and `vitriol-tui` (outside) points into it.

Trade-offs: one source of truth (good), but it makes the snapshot carry a crate that only a sibling
binary consumes, and it couples Officina's release cadence to the dashboard's. Recommendation:
**do §1.2 now, add a cross-crate parity test** (each side parses the other's `theme.rs` and compares
the VITRIOL literals — exactly the pattern the JS parity gate already uses), and only extract
`theme-core` if a third surface appears.

## 2. A VITRIOL *desktop* theme (the other possible reading)

If "as well" meant the desktop rather than the TUI: package the VITRIOL palette the way
`themes/kanagawa-aetherskiff/` and `themes/alka-officina/` are packaged — VSCode colour theme,
Plasma `.colors`, kitty/alacritty/zellij/termux, wallpapers, `install.sh`. Today `officina/theme/officina.json`
holds the palette for the pi theme, and the extensions carry a TS copy; a desktop package would need a
**single palette source of truth plus a generator** (one YAML/JSON → every surface), otherwise it is
five hand-maintained copies waiting to drift.

That is a genuinely different project (a generator + five output formats), and it would also be the
right home for the `#FF5F1F` → `#FFA000` correction.

## 3. Recommendation and the question

- **If the dashboard:** §1.2, plus the ORANGE fix, with a cross-crate parity test. It is mechanical,
  the pattern is proven in this session, and the gates come for free.
- **If the desktop:** §2 as its own plan (palette source + generator + five surfaces).

Which one? I would do the dashboard first regardless — it is the surface that is *already* out of
date, and it takes hours rather than days.

## 4. Both, then — and the order

**Owner (2026-11-18): "both actually. But I meant the dashboard, but a desktop theme would also be
cool."**

So: **A = dashboard** (the ask), **B = VITRIOL desktop theme** (the cool follow-up). A first, because
it is hours and it fixes a live inaccuracy; B second, because it needs a generator and a single
palette source.

### 4.1 The palette is copied four times inside the repo (plus zero desktop packages)

| copy | what it feeds |
|---|---|
| `officina/tui/src/theme.rs` | the chat TUI — now 3 palettes, the most complete source |
| `vitriol-tui/src/theme.rs` | the dashboard — **drifted** (`ORANGE #FF5F1F`) |
| `officina/theme/officina.json` | pi's own theme (`vars` + `colors` mapping, 18 vars) |
| `officina/.pi/extensions/_shared/vitriolum.ts` | extension widgets/sidebar (ANSI truecolour) |

Every new surface adds a copy; B would add six more (VSCode, Plasma, kitty, alacritty, zellij,
termux). That is the real reason to do B properly rather than by hand.

### 4.2 Workstream A — dashboard theming (already specified in §1.2)

Unchanged from §1.2, with one addition: fix `ORANGE` to `#FFA000` **first**, as its own commit, so
the correctness fix is not entangled with a 281-site refactor.

### 4.3 Workstream B — VITRIOL desktop theme, generated

**Single source of truth.** Add `officina/theme/palette.json` (inside the published subtree, so it
ships): every role for all three palettes, in one file. Then:

- a Rust test asserts `Theme::VITRIOL`/`AETHER`/`ALKAHEST` equal that file (the literal pin becomes
  data-driven, and a palette edit is one file);
- a cross-crate test asserts `vitriol-tui`'s palette equals the same file (kills the `ORANGE` class
  of drift permanently);
- the extension palette (`vitriolum.ts`) already has a parity gate — point it at the file too.

**Generator.** `scripts/gen-desktop-theme.mjs` (node, zero deps) reads `palette.json` and writes a
theme directory modelled on `themes/alka-officina/`:

| surface | output | note |
|---|---|---|
| VSCode | `vscode/<id>/package.json` + `themes/<name>-color-theme.json` | ~130 `colors` entries + tokenColors; the existing dirs are the template |
| Plasma 6 | `plasma/<Name>.colors` | `[Colors:*]` sections, `R,G,B` triples |
| kitty / alacritty / zellij / termux | one file each | trivial mappings |
| pi / Officina | `officina/theme/officina.json` | already exists; regenerating it keeps the two in step |
| wallpapers | leave to the owner | art is not generated |

`themes/` is **not a git repository**, so generated output is unversioned by design — the palette in
the repo is the source, and `gen-desktop-theme.mjs --out <dir>` can rebuild the directory at any
time. Name: `themes/vitriol/` (palette `vitriol`), with `--palette aether|alkahest` available so the
generator can also rebuild the two existing dirs if they ever drift.

Install: `install.sh` + `apply.sh` modelled on `themes/alka-officina/install.sh` (43 lines: copy
files, `plasma-apply-colorscheme`, kitty reload).

### 4.4 Sequence and gates

1. `ORANGE` fix in `vitriol-tui` (one line + a test that it differs from `RED` by a wide margin).
2. `officina/theme/palette.json` + the three parity tests (Rust, cross-crate, TS).
3. Workstream A: dashboard `Theme` refactor + key/env/persistence + the four gates.
4. Workstream B: generator + `themes/vitriol/` + install/apply scripts; verify by applying the
   VSCode theme and comparing a screenshot against the TUI.
5. Optional later: swap the two existing desktop dirs to generated output.

Nothing in B depends on A, but A is cheaper and its palette work (step 2) is B's foundation — so the
order above is the shortest path to both.

## 5. Shipped (2026-11-18, commit `5534f63`)

| piece | where |
|---|---|
| **one palette source** — 23 roles × 3 palettes | `officina/theme/palette.json` |
| parity gates | Rust (`themes_match_the_palette_file`), cross-crate dashboard test, vitest (`palette.json is the single source of truth`) |
| dashboard `Theme` + 3 palettes + 280-site rename | `vitriol-tui/src/theme.rs` |
| `ORANGE #FF5F1F → #FFA000` + warning-vs-critical distance test | `vitriol-tui/src/theme.rs` |
| dashboard switching: `VITRIOL_TUI_THEME`, `Shift+T`, `~/.vitriol/tui-theme`, footer name | `vitriol-tui/src/{main,ui}.rs` |
| desktop generator (VSCode, Plasma 6, kitty, alacritty, zellij, termux, README, apply.sh) | `scripts/gen-desktop-theme.mjs` (+ 4 tests) |
| activity ladder: compacting → **reasoning** → **building** → working | `officina/tui/src/tui/{state,layout}.rs` |

The owner's desktop and VSCode stay on **Kanagawa Aetherskiff**: the generator only writes under
`--out`, `apply.sh` is an explicit act, and nothing in the change touches live desktop config.
Generated package: `~/Desktop/Projects/themes/vitriol/` (9 files, JSON validated).

Gates at the end: Officina **364 cargo + 1471 vitest**, dashboard **153 cargo**, `fmt` clean on both,
zero new warnings, both binaries rebuilt (`~/.local/bin/officina` = `8573562f`, `vitriol-tui` release
at 5.7 MB). CI green — note the remote also received a concurrent `codex/avatar-startup-integration`
merge (`87f905c`) from another session; it applied cleanly and all suites above ran *after* it.

### Follow-ups worth knowing

1. `themes/` is not a git repo, so the generated package is unversioned by design — `palette.json` is
   the source. Consider `git init` there if you want history of the wallpaper era.
2. The generator can also rebuild `aether`/`alkahest` packages (`--palette`), but your existing
   `themes/kanagawa-aetherskiff` and `themes/alka-officina` were hand-built and are richer (GTK,
   Kvantum, wallpapers). Regenerating would overwrite them, so don't unless you mean to.
3. The dashboard's `USER_BG`/`FAINT`/`VIOLET`/`COLD_BLUE`/`SILVER`/`WATERMARK` roles are defined and
   pinned but not yet drawn anywhere — kept so the struct mirrors `palette.json` role for role.
