import { describe, expect, it } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  agentsSignature,
  buildDigest,
  digestSummary,
  findAgentsFiles,
  loadAgentsFiles,
  type AgentsFile,
} from "./discovery.ts";

const file = (path: string, text: string, mtime = 1): AgentsFile => ({ path, text, mtime });

describe("findAgentsFiles", () => {
  it("collects ancestors nearest-last (later overrides)", () => {
    const fs = new Set(["/a/AGENTS.md", "/a/b/AGENTS.md"]);
    expect(findAgentsFiles("/a/b/c", { exists: (p) => fs.has(p) })).toEqual([
      "/a/AGENTS.md",
      "/a/b/AGENTS.md",
    ]);
  });
});

describe("buildDigest", () => {
  const agents = file(
    "/repo/AGENTS.md",
    [
      "# VITRIOL",
      "Prose that should not be copied wholesale into the digest.",
      "",
      "## Rules",
      "- Never commit secrets",
      "- Run cargo test",
      "1. Keep invariants in plans",
      "plain prose line",
    ].join("\n"),
  );

  it("keeps headings and bullets, drops prose, counts rules", () => {
    const out = buildDigest([agents], { cwd: "/repo" });
    expect(out.text).toContain("### AGENTS.md");
    expect(out.text).toContain("## Rules");
    expect(out.text).toContain("- Run cargo test");
    expect(out.text).not.toContain("Prose that should not");
    expect(out.rules).toBe(3);
    expect(out.truncated).toBe(false);
  });

  it("caps the digest and says so", () => {
    const big = file(
      "/repo/AGENTS.md",
      Array.from({ length: 200 }, (_, i) => `- rule ${i} ${"x".repeat(80)}`).join("\n"),
    );
    const out = buildDigest([big], { cwd: "/repo", maxChars: 800 });
    expect(out.text.length).toBeLessThanOrEqual(900);
    expect(out.truncated).toBe(true);
    expect(out.text).toContain("truncated");
  });

  it("empty set → no digest and a clear summary", () => {
    const out = buildDigest([]);
    expect(out.text).toBe("");
    expect(digestSummary([], 0, 0)).toContain("none found");
  });
});

describe("reactivity signature", () => {
  it("changes when a mtime changes", () => {
    const a = [file("/x/AGENTS.md", "x", 100)];
    const b = [file("/x/AGENTS.md", "x", 200)];
    expect(agentsSignature(a)).not.toBe(agentsSignature(b));
    expect(agentsSignature(a)).toBe(agentsSignature([file("/x/AGENTS.md", "y", 100)]));
  });
});

describe("loadAgentsFiles", () => {
  it("honours OFFICINA_AGENTS_FILES and skips missing paths", () => {
    const dir = mkdtempSync(join(tmpdir(), "agents-md-"));
    const a = join(dir, "A.md");
    writeFileSync(a, "# A\n- rule\n");
    const files = loadAgentsFiles(dir, { OFFICINA_AGENTS_FILES: `${a}:/nope` } as NodeJS.ProcessEnv);
    expect(files).toHaveLength(1);
    expect(files[0].text).toContain("- rule");
    rmSync(dir, { recursive: true, force: true });
  });
});
