// Vitriolum — single source of the VITRIOL visual language for officina
// extensions (2026-08-31). Palette mirrors vitriol-tui/src/theme.rs (the
// canonical TUI definition) and officina/theme/officina.json (the pi theme);
// a vitest parity test (vitriolum.test.ts) fails the tree if the three
// drift apart. Before this module every extension re-declared its own ANSI
// constants, and the widget accent "honey" had drifted off-palette
// (#e15a1f); the brand accent is now the theme's antidote (#ff5f1f).
//
// Provenance: original work, this repo — constants ported in-house from
// vitriol-tui/src/theme.rs (same repo, Apache-2.0).

export const VITRIOLUM = {
  bg: "#0d1117", // substrate background (OSC 11 claim, TUI BG)
  panel: "#161b22", // panels / cards
  borderDim: "#21262d", // dim borders / darkGray
  substrate: "#ff4444", // errors, removals
  safety: "#39ff14", // success, additions, token flow
  solvent: "#00ffff", // links, folders, "alive" cyan
  sovereignty: "#ffd700", // gold: accent, headings, coupling
  antidote: "#ff5f1f", // warning orange; the brand widget accent
  coldBlue: "#2e5fa3", // borders
  violet: "#b294bb", // files, custom-message labels
  text: "#e0e0e0",
  gray: "#8b949e", // muted labels
  dimGray: "#5f6672", // dim
  darkGray: "#21262d",
  mercury: "#55606e", // idle mercury (ramp start)
  lightYellow: "#ffe066", // capacity ramp stop
  deepRed: "#8a1515", // capacity ramp end
  darkTeal: "#0b5e4c", // activity ramp start
  watermark: "#1c2634", // braille logo tint on bg
} as const;

export type VitriolumName = keyof typeof VITRIOLUM;

export function hexToRgb(hexCode: string): { r: number; g: number; b: number } {
  const n = parseInt(hexCode.replace("#", ""), 16);
  return { r: (n >> 16) & 0xff, g: (n >> 8) & 0xff, b: n & 0xff };
}

const ANSI_RESET_FG = "\x1b[39m";

/** Foreground SGR wrapper in a named Vitriolum color. */
export function fg(name: VitriolumName, s: string): string {
  const { r, g, b } = hexToRgb(VITRIOLUM[name]);
  return `\x1b[38;2;${r};${g};${b}m${s}${ANSI_RESET_FG}`;
}

/** Raw foreground SGR sequence for a named color (no reset). */
export function fgSeq(name: VitriolumName): string {
  const { r, g, b } = hexToRgb(VITRIOLUM[name]);
  return `\x1b[38;2;${r};${g};${b}m`;
}

// Convenience accents shared by widget-rendering extensions.
export const honey = (s: string) => fg("antidote", s); // brand accent
export const muted = (s: string) => fg("gray", s);
export const gold = (s: string) => fg("sovereignty", s);
export const ok = (s: string) => fg("safety", s);
export const err = (s: string) => fg("substrate", s);
