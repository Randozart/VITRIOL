// Native parity tests (2026-09-01): the Rust addon (officina/native) must be
// byte-identical to the JS fallbacks, or the fallbacks take over. Skips
// silently when index.node is absent (addon not built) — CI without cargo
// still passes, but then only the JS path is covered.
import { describe, expect, it } from "vitest";
import { getNative, nativeActive } from "./native.ts";
import { RAMPS, renderGauge } from "../vitriol-decode/braille.ts";
import { VITRIOLUM, hexToRgb } from "./vitriolum.ts";

// JS fallbacks — exact ports of the OfficinaSplit implementations in
// runtime/build-patch.mjs (kept in sync by the parity assertions below).
function jsStrip(line: string): string {
  return line
    .replace(/\x1b\[[0-9;?]*[a-zA-Z]/g, "")
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b_[^\x07\x1b]*(?:\x07|\x1b\\)/g, "");
}
function jsWidth(line: string): number {
  let w = 0;
  for (const ch of jsStrip(line)) {
    const cp = ch.codePointAt(0) ?? 0;
    w += cp >= 0x1100 && (cp <= 0x115f || (cp >= 0x2e80 && cp <= 0xa4cf) || (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xff00 && cp <= 0xff60)) ? 2 : 1;
  }
  return w;
}
function jsCut(line: string, width: number): string {
  let vis = 0;
  let out = "";
  let i = 0;
  while (i < line.length) {
    if (line[i] === "\x1b") {
      const rest = line.slice(i);
      const m = /^(\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b_[^\x07\x1b]*(?:\x07|\x1b\\))/.exec(rest);
      if (m) {
        out += m[1];
        i += m[1].length;
        continue;
      }
    }
    if (vis >= width) {
      i++;
      continue;
    }
    const cp = line.codePointAt(i) ?? 0;
    const ch = String.fromCodePoint(cp);
    const cw = cp >= 0x1100 && (cp <= 0x115f || (cp >= 0x2e80 && cp <= 0xa4cf) || (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xff00 && cp <= 0xff60)) ? 2 : 1;
    if (vis + cw > width) {
      i += ch.length;
      continue;
    }
    out += ch;
    vis += cw;
    i += ch.length;
  }
  return out;
}

const SAMPLES: Array<[string, number]> = [
  ["plain ascii", 6],
  ["\x1b[38;2;255;95;31mcolored text\x1b[0m", 7],
  ["\x1b]0;window title\x07after osc", 5],
  ["\x1b_APC payload\x1b\\after apc", 4],
  ["\x1b[1mwide 日本語 mix\x1b[0m", 8],
  ["한글 fullwidth ！ tail", 10],
  ["\x1b[31mpartial", 3],
  ["", 0],
  ["\x1bZlone escape", 5],
];

describe("officina-native", () => {
  const n = getNative();

  it("is present (warns when not built)", () => {
    // Not a hard failure: JS fallbacks are the correctness contract. But in
    // this workspace the addon should be built (npm run native:build).
    if (!nativeActive()) console.warn("[native-parity] addon not built — JS fallback only");
    expect(true).toBe(true);
  });

  (n ? it : it.skip)("stripAnsi matches JS", () => {
    for (const [line] of SAMPLES) expect(n!.stripAnsi(line)).toBe(jsStrip(line));
  });

  (n ? it : it.skip)("visibleWidth matches JS", () => {
    for (const [line] of SAMPLES) expect(n!.visibleWidth(line)).toBe(jsWidth(line));
  });

  (n ? it : it.skip)("cutLine matches JS", () => {
    for (const [line, w] of SAMPLES) {
      expect(n!.cutLine(line, w)).toBe(jsCut(line, w));
      expect(n!.cutLine(line, 0)).toBe(jsCut(line, 0));
      expect(n!.cutLine(line, 100)).toBe(jsCut(line, 100));
    }
  });

  (n ? it : it.skip)("renderGauge matches TS renderer for all ramps", () => {
    const { r, g, b } = hexToRgb(VITRIOLUM.gray);
    for (const ramp of [RAMPS.capacity, RAMPS.activity, RAMPS.mercury]) {
      for (const ratio of [0, 0.08, 0.25, 0.5, 0.75, 1]) {
        for (const cells of [1, 4, 8, 10]) {
          const native = n!.renderGauge(ramp.name!, ratio, cells, r, g, b);
          const js = renderGauge(ramp, ratio, cells);
          expect(native).toBe(js);
        }
      }
    }
  });

  (n ? it : it.skip)("mergeSplitRows matches OfficinaSplit.render loop", () => {
    function jsMerge(mainLines: string[], mainW: number, sbLines: string[], sbW: number, sbPad: number, gap: number, bg: string, reset: string): string[] {
      const total = Math.max(mainLines.length, sbLines.length + sbPad);
      const out: string[] = [];
      for (let r = 0; r < total; r++) {
        let left = mainLines[r] ?? "";
        if (jsWidth(left) > mainW) left = jsCut(left, mainW);
        let line = left + " ".repeat(Math.max(0, mainW - jsWidth(left)));
        if (sbW > 0) {
          let right = r >= sbPad ? sbLines[r - sbPad] ?? "" : "";
          if (jsWidth(right) > sbW) right = jsCut(right, sbW);
          const pad = Math.max(0, sbW - jsWidth(right));
          line += " ".repeat(gap) + bg + right + " ".repeat(pad) + reset;
        }
        out.push(line);
      }
      return out;
    }
    const bg = "\x1b[48;2;22;27;34m";
    const reset = "\x1b[0m";
    const main = ["hello", "\x1b[31ma red line that overflows the column width badly\x1b[0m", "한글 wide row"];
    const sb = ["\x1b[38;2;255;215;0m◈ coupling\x1b[0m", "ctx ⣿⣿⣀⣀ 42%", ""];
    for (const [mw, sw, sp] of [[12, 20, 0], [42, 42, 0], [80, 42, 2], [5, 8, 1]] as Array<[number, number, number]>) {
      const native = n!.mergeSplitRows({ mainLines: main, mainW: mw, sbLines: sb, sbW: sw, sbPad: sp, gap: 1, bg, reset });
      expect(native).toEqual(jsMerge(main, mw, sb, sw, sp, 1, bg, reset));
    }
    // hidden sidebar case
    const hidden = n!.mergeSplitRows({ mainLines: main, mainW: 40, sbLines: [], sbW: 0, sbPad: 0, gap: 1, bg, reset });
    expect(hidden).toEqual(jsMerge(main, 40, [], 0, 0, 1, bg, reset));
  });

  (n ? it : it.skip)("ramp stops match the Vitriolum palette", () => {
    const rgb = (name: keyof typeof VITRIOLUM) => {
      const { r, g, b } = hexToRgb(VITRIOLUM[name]);
      return [r, g, b];
    };
    const WHITE = [255, 255, 255]; // capacity ramp start (theme.rs literal)
    // [at×1000, r, g, b] per stop, ascending — must mirror braille.rs ramps.
    const expected: Record<string, number[][]> = {
      capacity: [
        [0, ...WHITE],
        [250, ...rgb("lightYellow")],
        [500, ...rgb("antidote")],
        [750, ...rgb("substrate")],
        [1000, ...rgb("deepRed")],
      ],
      activity: [
        [0, ...rgb("darkTeal")],
        [500, ...rgb("safety")],
        [1000, ...rgb("solvent")],
      ],
      mercury: [
        [0, ...rgb("mercury")],
        [1000, ...rgb("solvent")],
      ],
    };
    for (const [name, stops] of Object.entries(expected)) {
      expect(n!.rampStops(name)).toEqual(stops.flat());
    }
  });
});
