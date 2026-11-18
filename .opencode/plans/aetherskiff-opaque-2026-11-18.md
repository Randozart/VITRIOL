# Aetherskiff, opaque variant

**Date:** 2026-11-18 19:00
**Mode:** PLAN (research only — nothing written outside this file)
**Owner:** "make me an aetherskiff theme without transparent background. It doesn't quite work with the
transparent background I have normally."

## 1. Where the transparency actually comes from — and where it does not

| layer | file | transparent? |
|---|---|---|
| **kitty** | `themes/kanagawa-aetherskiff/terminal/kitty/kitty.conf:24` | **yes — `background_opacity 0.5`, `background_blur 1`** ("heavy frosted glass", by design) |
| **alacritty** | `terminal/alacritty/alacritty.toml:13` | yes — `opacity = 0.65` |
| Kvantum (menus/widgets) | `kvantum/Kanagawa/Kanagawa.kvconfig` | **no** — `translucent_windows=false`, `blurring=false`, `reduce_window_opacity=0` |
| GTK | `gtk/Kanagawa-Dark` | no |
| Plasma scheme | `plasma/…`, `color-schemes/…` | no (panel translucency is a global KWin setting, not this theme) |

So the frosted look is **two lines in one file**, not a property of the palette. Your live kitty
includes it here:

```
~/.config/kitty/kitty.conf:1: include themes/kanagawa_wave.conf
```

## 2. Simplest possible fix (you can do this in ten seconds)

In `~/.config/kitty/themes/kanagawa_wave.conf`:

```diff
-# Background opacity — heavy frosted glass
-background_opacity 0.5
-dynamic_background_opacity yes
-background_blur 1
+# Background — opaque (the scheme is designed for a solid ground)
+background_opacity 1.0
+dynamic_background_opacity no
+background_blur 0
```

Colours are untouched: removing the alpha channel does not change a single hex value, it only stops
whatever is behind the terminal from showing through — which is what makes the muted greys (and the
scheme's `#1F1F28` sumi ink) read as intended.

## 3. What I would rather ship (BUILD)

1. **A sibling file, not an edit of the original**: `themes/kanagawa-aetherskiff/terminal/kitty/
   kanagawa-opaque.conf` — the same file with the three lines above, so `themes/` keeps the author's
   frosted theme and gains an opaque variant you can include instead:
   ```
   include themes/kanagawa_opaque.conf      # ~/.config/kitty/kitty.conf
   ```
2. **`alacritty-opaque.toml`** the same way (`opacity = 1.0`) for parity, even if you do not use it.
3. **The generator grows `--opaque`** (`scripts/gen-desktop-theme.mjs`): it emits no opacity keys
   today, so generated themes are already opaque — the flag would make that explicit and put a
   `background_opacity 1.0` line in the kitty output, so nobody has to guess which the theme intends.
4. **A line in the theme README** stating which variant assumes a solid ground, so the choice is
   documented rather than remembered.

## 4. Trade-offs

- **Blur off** costs nothing visually once the ground is opaque (there is nothing left to blur) and
  saves a compositor pass — worth doing together.
- **The frosted variant is not "broken"**: it is the theme's stated identity ("iced glass"), so it
  stays as the default in `themes/`; the opaque one is the variant for your setup.
- If you ever want *partial* translucency back, `background_opacity 0.85` with blur off is the
  sweet spot for a dark scheme: enough ground to keep the muted greys legible, still a hint of depth.

Ready to write the two files and the `--opaque` flag on your word (switch to BUILD) — or apply the
four-line diff above now and skip the plumbing.
