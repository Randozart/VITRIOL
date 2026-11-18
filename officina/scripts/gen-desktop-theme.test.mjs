// Desktop theme generator (2026-11-18): one palette source, six surfaces.
//
// The gate that matters is fidelity: every emitted file must carry the values in
// `officina/theme/palette.json`, and the generator must never touch a live
// desktop config (it only writes under --out).
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..", "..");
const script = join(repo, "scripts", "gen-desktop-theme.mjs");
const palette = JSON.parse(
  readFileSync(join(repo, "officina", "theme", "palette.json"), "utf-8"),
);

const dirs = [];
function generate(name) {
  const out = mkdtempSync(join(tmpdir(), `theme-${name}-`));
  dirs.push(out);
  execFileSync(process.execPath, [script, "--palette", name, "--out", out], { stdio: "pipe" });
  return out;
}

afterAll(() => {
  for (const d of dirs) rmSync(d, { recursive: true, force: true });
});

describe("gen-desktop-theme", () => {
  it("emits every surface for a palette", () => {
    const out = generate("vitriol");
    const files = [
      "vscode/officina-vitriol/package.json",
      "vscode/officina-vitriol/themes/officina-vitriol-color-theme.json",
      "plasma/VITRIOLDark.colors",
      "terminal/kitty/vitriol.conf",
      "terminal/alacritty/vitriol.toml",
      "terminal/zellij/vitriol.kdl",
      "terminal/termux/colors.properties",
      "README.md",
    ];
    for (const rel of files) {
      expect(() => readFileSync(join(out, rel)), rel).not.toThrow();
    }
    // Nothing may be written outside the requested directory (apply.sh is part
    // of the package, not an action: it only runs when the operator runs it).
    expect(readdirSync(out).sort()).toEqual([
      ".generated-from",
      "README.md",
      "apply.sh",
      "plasma",
      "terminal",
      "vscode",
    ]);
    expect(readFileSync(join(out, "apply.sh"), "utf-8")).toContain("plasma-apply-colorscheme");
  });

  it("carries the palette's own values into every surface", () => {
    const out = generate("vitriol");
    const p = palette.palettes.vitriol;
    const hex = (role) => String(p[role]).toUpperCase();
    const triple = (role) => {
      const h = String(p[role]);
      return [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16)).join(",");
    };

    const kitty = readFileSync(join(out, "terminal/kitty/vitriol.conf"), "utf-8");
    expect(kitty).toContain(`background ${hex("bg")}`);
    expect(kitty).toContain(`foreground ${hex("text")}`);
    expect(kitty.match(/^color\d+ /gm)?.length).toBe(16);

    const plasma = readFileSync(join(out, "plasma/VITRIOLDark.colors"), "utf-8");
    expect(plasma).toContain(`ForegroundActive=${triple("gold")}`);
    expect(plasma).toContain(`ForegroundNegative=${triple("red")}`);

    const vscode = JSON.parse(
      readFileSync(join(out, "vscode/officina-vitriol/themes/officina-vitriol-color-theme.json"), "utf-8"),
    );
    expect(vscode.colors["editor.background"]).toBe(hex("bg"));
    expect(vscode.colors["editor.foreground"]).toBe(hex("text"));
    expect(vscode.colors["terminal.ansiRed"]).toBe(hex("red"));
    expect(vscode.tokenColors.length).toBeGreaterThan(5);
  });

  it("uses the same generator for every palette", () => {
    for (const name of ["vitriol", "aether", "alkahest"]) {
      const out = generate(name);
      const kitty = readFileSync(join(out, `terminal/kitty/${name}.conf`), "utf-8");
      expect(kitty).toContain(String(palette.palettes[name].bg).toUpperCase());
    }
  });

  it("refuses an unknown palette without writing anything", () => {
    expect(() => generate("not-a-palette")).toThrow();
  });
});
