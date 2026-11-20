// Mode registry gates (2026-11-19): the mode list grew to 8 — pin the
// invariants that keep the TAB cycle and the mode-menu readable.
//
// I7: existing modes are additive — new modes appended after the original
//     five, order stable (TAB muscle memory).
// I7b: mode colours pairwise-distinct + readable (>= 3:1 on the vitriol bg)
//      — a collision silently makes two badges indistinguishable.

import { describe, expect, it } from "vitest";
import {
  DEFAULT_MODES,
  allModes,
  getModeDef,
  nextMode,
  setAgentMode,
} from "./agent-mode.ts";

// vitriol theme ground — the badge colours must read on it.
const VITRIOL_BG = [0x0d, 0x11, 0x17] as const;

function hexToRgb(hex: string): [number, number, number] {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) throw new Error(`not a hex colour: ${hex}`);
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

function relativeLuminance([r, g, b]: readonly number[]): number {
  const f = (c: number) => {
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function contrast(a: readonly number[], b: readonly number[]): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

describe("mode registry gates", () => {
  it("the original five keep their order (TAB muscle memory, I7)", () => {
    const names = DEFAULT_MODES.map((m) => m.name);
    expect(names.slice(0, 5)).toEqual([
      "build",
      "grind",
      "steer",
      "inquire",
      "plan",
    ]);
    // New modes are appended, never inserted.
    expect(names.slice(5)).toEqual(["conversation", "research", "pipeline"]);
  });

  it("mode colours are pairwise distinct", () => {
    const colours = DEFAULT_MODES.map((m) => m.color);
    expect(new Set(colours).size).toBe(colours.length);
  });

  it("every mode colour is readable on the vitriol ground (>= 3:1)", () => {
    for (const m of DEFAULT_MODES) {
      const fg = m.color.startsWith("#")
        ? hexToRgb(m.color)
        : hexToRgb("#ffd700"); // named colours resolve in vitriolum; gold is the fallback check
      const c = contrast(fg, VITRIOL_BG);
      expect(c).toBeGreaterThanOrEqual(3);
    }
  });

  it("every mode has a non-empty directive or enterDirective", () => {
    for (const m of DEFAULT_MODES) {
      expect(m.label.length).toBeGreaterThan(0);
      expect((m.directive ?? m.enterDirective ?? "").length).toBeGreaterThan(
        20,
      );
    }
  });

  it("research and pipeline gate writes; conversation does not", () => {
    const by = (n: string) => DEFAULT_MODES.find((m) => m.name === n)!;
    expect(by("research").blockNonMdWrites).toBe(true);
    // pipeline writes plans + code, so it must NOT block
    expect(by("pipeline").blockNonMdWrites).toBeFalsy();
    expect(by("conversation").blockNonMdWrites).toBeFalsy();
  });

  it("TAB cycle walks all modes and wraps", () => {
    // The registry is module state — reset to defaults explicitly.
    setAgentMode("build");
    const seen: string[] = [];
    let cur = getModeDef();
    for (let i = 0; i < DEFAULT_MODES.length; i++) {
      seen.push(cur.name);
      cur = nextMode(cur.name);
    }
    expect(seen).toEqual(DEFAULT_MODES.map((m) => m.name));
    expect(cur.name).toBe("build"); // wrapped
    expect(allModes().length).toBe(8);
  });
});
