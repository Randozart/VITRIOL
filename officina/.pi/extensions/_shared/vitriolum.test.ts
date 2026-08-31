// Parity gate for the Vitriolum palette: the TUI (vitriol-tui/src/theme.rs),
// the pi theme (theme/officina.json), and the extension palette
// (_shared/vitriolum.ts) must agree. This test fails if any of the three
// drifts — the whole point of the 2026-08-31 unification.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { VITRIOLUM } from "./vitriolum.ts";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "..", "..", "..", "..");

const themeRs = readFileSync(join(repo, "vitriol-tui", "src", "theme.rs"), "utf-8");
const watermarkRs = readFileSync(join(repo, "vitriol-tui", "src", "watermark.rs"), "utf-8");
const officinaJson = readFileSync(join(repo, "officina", "theme", "officina.json"), "utf-8");

// theme.rs encodes colors as Color::Rgb(0xRR, 0xGG, 0xBB) triplets.
function themeRsHexes(): Set<string> {
  const set = new Set<string>();
  const rx = /Color::Rgb\(0x([0-9A-Fa-f]{2}),\s*0x([0-9A-Fa-f]{2}),\s*0x([0-9A-Fa-f]{2})\)/g;
  for (const m of themeRs.matchAll(rx)) {
    set.add(`#${m[1]}${m[2]}${m[3]}`.toLowerCase());
  }
  return set;
}

describe("Vitriolum palette parity", () => {
  it("every palette color exists in the TUI theme.rs", () => {
    const rs = themeRsHexes();
    for (const [name, hex] of Object.entries(VITRIOLUM)) {
      // officina-only extensions (officina.json vars / watermark.rs), not in theme.rs:
      if (name === "violet" || name === "dimGray" || name === "watermark") continue;
      expect(rs.has(hex.toLowerCase()), `${name} ${hex} missing from theme.rs`).toBe(true);
    }
  });

  it("violet matches the officina.json var it came from", () => {
    expect(officinaJson).toContain('"violet": "#b294bb"');
  });

  it("core accents match officina.json vars", () => {
    // officina.json defines the same names with the same values.
    const corePairs: Array<[keyof typeof VITRIOLUM, string]> = [
      ["substrate", "substrate"],
      ["safety", "safety"],
      ["solvent", "solvent"],
      ["sovereignty", "sovereignty"],
      ["antidote", "antidote"],
      ["coldBlue", "coldBlue"],
      ["violet", "violet"],
      ["text", "text"],
      ["gray", "gray"],
      ["dimGray", "dimGray"],
      ["darkGray", "darkGray"],
    ];
    for (const [pal, jsonName] of corePairs) {
      const expected = `"${jsonName.toLowerCase()}": "${VITRIOLUM[pal].toLowerCase()}"`;
      expect(officinaJson.toLowerCase()).toContain(expected);
    }
  });

  it("background and watermark tint match the TUI", () => {
    expect(VITRIOLUM.bg).toBe("#0d1117");
    // watermark tint lives in watermark.rs (theme.rs-adjacent constant)
    expect(watermarkRs.toLowerCase()).toContain(VITRIOLUM.watermark.slice(1));
  });
});
