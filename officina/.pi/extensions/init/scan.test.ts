// Tests for init's pure scan + seed builder (parity plan phase 1.4).
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { buildMemorySeed, scanProject } from "./scan.ts";

let proj: string;

beforeAll(() => {
  proj = mkdtempSync(join(tmpdir(), "init-scan-"));
  mkdirSync(join(proj, "src"));
  mkdirSync(join(proj, ".git"));
  writeFileSync(join(proj, "package.json"), JSON.stringify({ scripts: { build: "tsc", test: "vitest" } }));
  writeFileSync(join(proj, "README.md"), "# x\n");
  writeFileSync(join(proj, "AGENTS.md"), "rules\n");
  writeFileSync(join(proj, "a.ts"), "export {};\n");
  writeFileSync(join(proj, "b.ts"), "export {};\n");
  writeFileSync(join(proj, "src/c.py"), "x = 1\n");
  writeFileSync(join(proj, "Cargo.toml"), "[package]\nname=\"x\"\n");
});

afterAll(() => rmSync(proj, { recursive: true, force: true }));

describe("scanProject", () => {
  it("detects manifests, docs, git, languages", () => {
    const s = scanProject(proj);
    expect(s.name).toBe(proj.split("/").pop());
    expect(s.buildFiles).toEqual(["package.json", "Cargo.toml"]);
    expect(s.docs).toEqual(expect.arrayContaining(["README.md", "AGENTS.md"]));
    expect(s.hasGit).toBe(true);
    expect(s.topDirs).toEqual(["src"]);
    expect(s.languages[0]).toMatch(/TypeScript/);
    expect(s.languages.join(",")).toMatch(/Python/);
  });

  it("infers commands only from real manifests + real scripts", () => {
    const s = scanProject(proj);
    // package.json has build+test scripts -> npm commands suggested
    expect(s.commands).toContain("npm run build");
    expect(s.commands).toContain("npm test");
    // Cargo.toml present -> cargo commands suggested
    expect(s.commands).toContain("cargo test");
  });

  it("skips npm scripts that do not exist", () => {
    writeFileSync(join(proj, "package.json"), JSON.stringify({ scripts: { build: "tsc" } }));
    const s = scanProject(proj);
    expect(s.commands).toContain("npm run build");
    expect(s.commands).not.toContain("npm test");
    expect(s.commands).not.toContain("npm run dev");
  });

  it("survives an empty directory", () => {
    const empty = mkdtempSync(join(tmpdir(), "init-empty-"));
    try {
      const s = scanProject(empty);
      expect(s.buildFiles).toEqual([]);
      expect(s.languages).toEqual([]);
      expect(s.hasGit).toBe(false);
      expect(buildMemorySeed(s, "t").length).toBeGreaterThan(50);
    } finally {
      rmSync(empty, { recursive: true, force: true });
    }
  });
});

describe("buildMemorySeed", () => {
  it("renders shape, toolchain, conventions sections", () => {
    const s = scanProject(proj);
    const md = buildMemorySeed(s, "2026-09-27 15:45");
    expect(md).toContain("# " + s.name + " — project memory");
    expect(md).toContain("Seeded by /init 2026-09-27 15:45");
    expect(md).toContain("## Shape");
    expect(md).toContain("## Toolchain");
    expect(md).toContain("## Conventions");
    expect(md).toContain("`cargo test`");
  });
});
