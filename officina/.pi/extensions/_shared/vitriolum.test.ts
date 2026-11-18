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
// The TUI sources live INSIDE the officina tree (tui/src/…) — the same
// relative path works in the monorepo and in the standalone snapshot.
const tuiRoot = join(here, "..", "..", "..");

const themeRs = readFileSync(join(tuiRoot, "tui", "src", "theme.rs"), "utf-8");
const watermarkRs = readFileSync(join(tuiRoot, "tui", "src", "watermark.rs"), "utf-8");
const officinaJson = readFileSync(join(tuiRoot, "theme", "officina.json"), "utf-8");

// theme.rs encodes colours as `Color::Rgb(0xRR, 0xGG, 0xBB)` triplets — or, since
// 2026-11-18, through the const-friendly `c_rgb(...)` helper the theme STATICS use.
// The parity intent is unchanged: every palette colour must appear in theme.rs.
function themeRsHexes(): Set<string> {
  // Only the VITRIOL theme block: since 2026-11-18 theme.rs carries three
  // palettes, and a colour that only exists in Aether must NOT satisfy the
  // parity check for the original scheme.
  const start = themeRs.indexOf("pub static VITRIOL: Theme");
  const end = themeRs.indexOf("pub static AETHER: Theme");
  const block = start >= 0 && end > start ? themeRs.slice(start, end) : themeRs;
  const set = new Set<string>();
  const rx = /(?:Color::Rgb|c_rgb)\(0x([0-9A-Fa-f]{2}),\s*0x([0-9A-Fa-f]{2}),\s*0x([0-9A-Fa-f]{2})\)/g;
  for (const m of block.matchAll(rx)) {
    set.add(`#${m[1]}${m[2]}${m[3]}`.toLowerCase());
  }
  return set;
}

describe("Vitriolum palette parity", () => {
  it("every palette color exists in the TUI theme.rs", () => {
    const rs = themeRsHexes();
    for (const [name, hex] of Object.entries(VITRIOLUM)) {
      // officina-only extensions (officina.json vars / watermark.rs), not in theme.rs:
      if (name === "violet" || name === "dimGray") continue;
      if (name === "watermark") {
        expect(rs.has(hex.toLowerCase()), `watermark ${hex} missing from theme.rs`).toBe(true);
        continue;
      }
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
    // watermark tint: theme.rs declares WATERMARK; watermark.rs mirrors it
    expect(themeRsHexes().has(VITRIOLUM.watermark.toLowerCase())).toBe(true);
    expect(watermarkRs.toLowerCase()).toContain("theme::watermark");
  });
});

// 2026-11-18: the palettes have ONE source of truth — officina/theme/palette.json.
// The JS palette (widgets/sidebar) must equal its `vitriol` section exactly, so a
// colour cannot be edited in one place and left stale in the other.
describe("palette.json is the single source of truth", () => {
  const jsonPath = join(tuiRoot, "theme", "palette.json");
  const palette = JSON.parse(readFileSync(jsonPath, "utf-8")) as {
    palettes: Record<string, Record<string, unknown>>;
  };

  const roleFor: Record<string, string> = {
    bg: "bg",
    panel: "panel",
    borderDim: "border_dim",
    substrate: "red",
    safety: "green",
    solvent: "cyan",
    sovereignty: "gold",
    antidote: "orange",
    coldBlue: "cold_blue",
    violet: "violet",
    text: "text",
    gray: "muted",
    dimGray: "faint",
    darkGray: "border_dim",
    mercury: "mercury",
    lightYellow: "light_yellow",
    deepRed: "deep_red",
    darkTeal: "dark_teal",
    watermark: "watermark",
  };

  it("every Vitriolum colour equals the vitriol palette", () => {
    const vitriol = palette.palettes.vitriol!;
    for (const [name, hex] of Object.entries(VITRIOLUM)) {
      const role = roleFor[name];
      expect(role, `${name} has no palette role`).toBeTruthy();
      expect(String(vitriol[role]).toLowerCase(), `${name} → ${role}`).toBe(hex.toLowerCase());
    }
  });

  it("every palette has the full role set", () => {
    for (const [name, pal] of Object.entries(palette.palettes)) {
      for (const role of ["bg", "panel", "green", "gold", "red", "text", "muted", "faint"]) {
        expect(pal[role], `${name}.${role}`).toMatch(/^#[0-9a-f]{6}$/);
      }
      expect(Array.isArray(pal.fire), `${name}.fire`).toBe(true);
    }
  });
});
