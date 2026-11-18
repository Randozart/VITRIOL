# Officina themes

Three palettes ship with the TUI, selected at runtime. The colour scheme is data
(`officina/tui/src/theme.rs`), so adding a fourth is a `Theme` literal, not a
rewiring of the renderer.

## Switch

```
/theme              # show the active theme and the names
/theme vitriol      # gold on GitHub-dark — the original (default)
/theme aether       # Kanagawa Wave (sumi ink, carp yellow)
/theme alkahest     # Alka Neon (swirl yellow, electric accents)
```

The choice is persisted in `~/.vitriol/officina/tui-theme` and applied at
startup. A throwaway run can preview a theme without touching it:

```bash
OFFICINA_TUI_THEME=aether officina
```

Precedence: `OFFICINA_TUI_THEME` → saved file → `vitriol`.

## The palettes

| role | vitriol | aether | alkahest |
|---|---|---|---|
| ground | `#0D1117` | `#1F1F28` sumi ink | `#0D1117` |
| panel | `#161B22` | `#16161D` | `#161B22` |
| text | `#E0E0E0` | `#DCD7BA` fuji | `#E0E0E0` |
| secondary | `#8B949E` | `#C8C093` old white | `#8B949E` |
| faint (comments only) | `#6E7681` | `#727169` katana | `#6E7681` |
| accent | `#FFD700` gold | `#E6C384` carp yellow | `#DFFF00` swirl |
| success | `#3FB950` | `#98BB6C` | `#39FF14` neon |
| info / streaming | `#00FFFF` | `#7FB4CA` | `#44AAFF` |
| warning | `#FFA000` | `#FFA066` | `#FFD700` |
| critical | `#F85149` | `#E82424` | `#FF4444` |
| special | `#B294BB` violet | `#957FB8` wave violet | `#FF00FF` magenta |

`aether` follows the Kanagawa Wave desktop theme and `alkahest` the Alka Officina
theme — the same palettes the author's editor, Plasma and terminal use — so the
TUI matches the rest of the desktop. The values are inlined in `theme.rs`; nothing
here depends on those files being present.

## What a theme owns

Beyond the flat roles, each `Theme` carries:

- **`fire`** — the cold→hot gradient behind the flames and the text tint. A palette
  swap that ignored it would leave Kanagawa glowing GitHub-orange.
- **`syntax`** — the syntect theme for fenced code. `vitriol` uses
  `base16-ocean.dark`; the other two use embedded tmThemes generated from the same
  palettes (`officina/assets/syntect/*.tmTheme`).
- the gauge ramps (`BrailleRamp`) are derived from the roles, so gauges follow the
  theme automatically.

## Invariants the test suite enforces

1. Every role is defined in every theme (a `Reset` or background-coloured role
   fails the build).
2. Readability: body text ≥ 7:1 contrast on the ground, secondary ≥ 3.5:1.
3. No call site reads a colour as a compile-time constant — a `theme::[A-Z]` read
   without `()` fails a source-scanning test, because the failure mode is a
   *silently unthemed* widget that no rendering assertion would notice.
4. Fire ramps get warmer toward the hot end and never repeat a colour.
5. The three themes are actually distinct (ground, accent and success differ).

## Adding a theme

1. Add a `pub static NAME: Theme = Theme { … };` with every field set.
2. Append it to `ALL` and (optionally) an alias in `by_name`.
3. Add a tmTheme under `officina/assets/syntect/` if it needs its own code colours,
   and point `syntax` at it.
4. `cargo test theme_suite` — the completeness, contrast and distinctness tests
   will tell you what is missing.
