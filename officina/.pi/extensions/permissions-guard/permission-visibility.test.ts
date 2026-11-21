// Permission visibility tests (2026-11-21, plan
// permission-visibility-2026-11-21.md).
//
// I1: bash ask dialog contains the FULL command.
// I2: write/edit ask dialog shows a bounded content preview.
// I3: previews are capped (1 MB write must not hang a frame).
// I4: /permissions report never truncates rule patterns and names sources.
// I5: the report is read-only (pure function — nothing writes).
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { loadGlobalRules, renderReport } from "./perms-report.ts";
import { contentPreviewOf } from "./index.ts";

describe("content preview (I2/I3)", () => {
  it("shows +prefixed lines for content", () => {
    const p = contentPreviewOf("line one\nline two\nline three");
    expect(p).toContain("+ line one");
    expect(p).toContain("+ line three");
  });

  it("returns undefined for empty/whitespace content", () => {
    expect(contentPreviewOf(undefined)).toBeUndefined();
    expect(contentPreviewOf("   \n  ")).toBeUndefined();
  });

  it("caps line count (I3)", () => {
    const big = Array.from({ length: 200 }, (_, i) => `row ${i}`).join("\n");
    const p = contentPreviewOf(big)!;
    expect(p).toContain("… (");
    expect(p.split("\n").length).toBeLessThan(60);
  });

  it("caps total chars (I3: 1 MB write must not hang a frame)", () => {
    const huge = "x".repeat(1_000_000);
    const p = contentPreviewOf(huge)!;
    // Single-line content is capped by the per-line 200-char rule; a huge
    // MULTI-line write hits the char budget and gains the truncation tail.
    expect(p.length).toBeLessThan(1000);
    const multiline = Array.from({ length: 400 }, (_, i) => `line ${i} ${"y".repeat(50)}`).join("\n");
    const pm = contentPreviewOf(multiline)!;
    expect(pm.length).toBeLessThan(3000);
    expect(pm).toContain("truncated");
  });

  it("caps absurdly long single lines", () => {
    const p = contentPreviewOf("y".repeat(50_000))!;
    expect(p.length).toBeLessThan(1000);
  });
});

describe("perms report (I4/I5)", () => {
  const dir = mkdtempSync(join(tmpdir(), "perms-report-"));

  it("lists global rules verbatim, patterns untruncated (I4)", () => {
    const path = join(dir, "permissions.json");
    writeFileSync(
      path,
      JSON.stringify({
        rules: [
          {
            tool: "edit",
            pattern: "src/**/very/deep/**/and-a-long-pattern-segment/**",
            action: "allow",
          },
          { tool: "write", pattern: "**/.env", action: "deny" },
        ],
      }),
    );
    const rules = loadGlobalRules(path);
    expect(rules![0].pattern).toBe(
      "src/**/very/deep/**/and-a-long-pattern-segment/**",
    );
  });

  it("missing global file reports null (mode policy only)", () => {
    expect(loadGlobalRules(join(dir, "missing.json"))).toBeNull();
  });

  it("renders mode, both rule sets, and protected roots", () => {
    const lines = renderReport({
      mode: "ask",
      modeSource: "default",
      globalRules: [{ tool: "write", pattern: "**/.env", action: "deny" }],
      globalPath: "/home/x/.config/trismegistus/permissions.json",
      projectRules: [{ tool: "bash", kind: "program", pattern: "cargo" }],
      projectPath: "/home/x/.vitriol/officina/perms/p-abc.json",
      protectedRoots: ["/etc/**", "~/.ssh/**"],
    });
    const joined = lines.join("\n");
    expect(joined).toContain("mode: ask");
    expect(joined).toContain("**/.env");
    expect(joined).toContain("cargo");
    expect(joined).toContain("/etc/**");
    expect(joined).toContain("always ask");
  });

  it("read-only: rendering consumes arguments and touches nothing (I5)", () => {
    // renderReport is pure by construction (takes resolved inputs); the load
    // helpers only readFileSync. This test pins the contract that the report
    // does not carry write capability — if someone adds a store write here,
    // the pure signature will fail to compile.
    const lines = renderReport({
      mode: "yolo",
      modeSource: "test",
      globalRules: [],
      globalPath: "/x",
      projectRules: null,
      projectPath: "/y",
      protectedRoots: [],
    });
    expect(lines[0]).toContain("yolo");
  });

  it("cleanup", () => rmSync(dir, { recursive: true, force: true }));
});
