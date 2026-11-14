import { describe, expect, it } from "vitest";
import { renderReport, unifiedDiff, type CodeReport } from "./report.ts";

describe("unifiedDiff", () => {
  it("is empty for identical text", () => {
    expect(unifiedDiff("a\nb\n", "a\nb\n", "x.ts")).toBe("");
  });

  it("marks changed lines with -/+ and keeps context", () => {
    const d = unifiedDiff("a\nb\nc\n", "a\nB\nc\n", "x.ts");
    expect(d).toContain("--- x.ts");
    expect(d).toContain("+++ x.ts");
    expect(d).toContain("-b");
    expect(d).toContain("+B");
    expect(d).toContain(" a");
  });

  it("caps oversized inputs instead of hanging", () => {
    const big = Array.from({ length: 2000 }, (_, i) => `line ${i}`).join("\n");
    const d = unifiedDiff(big, `${big}\nnew`, "x.ts");
    expect(d).toContain("file too large to diff");
  });
});

describe("renderReport", () => {
  const base = (): CodeReport => ({
    formatted: [],
    syntax: [],
    gates: { ran: false, ok: true, outcomes: [] },
    praetor: [],
    budgetBytes: 1400,
  });

  it("reports formatting changes and diffs", () => {
    const r = base();
    r.formatted.push({
      file: "a.ts",
      tool: "prettier",
      changed: true,
      diff: "-old\n+new",
    });
    const text = renderReport(r);
    expect(text).toContain("1 file(s) formatted — 1 changed");
    expect(text).toContain("prettier a.ts (reformatted)");
    expect(text).toContain("-old");
  });

  it("shows syntax failures, gate failures and praetor advisories", () => {
    const r = base();
    r.formatted.push({ file: "a.py", tool: "ruff format", changed: false });
    r.syntax.push({ file: "a.py", ok: false, output: "SyntaxError: bad" });
    r.gates = {
      ran: true,
      ok: false,
      outcomes: [{ id: "typecheck", label: "typecheck", ok: false, message: "TS2304" }],
    };
    r.praetor.push({ file: "a.py", line: 4, message: "advisory" });
    const text = renderReport(r);
    expect(text).toContain("✗ syntax a.py");
    expect(text).toContain("✗ 1 gate(s) failing");
    expect(text).toContain("⚠ praetor: 1");
  });

  it("is silent-about-nothing sane when no formatter applies", () => {
    const text = renderReport(base());
    expect(text).toContain("no formatter applies");
    expect(text).toContain("no project gates detected");
  });

  it("truncates to the budget", () => {
    const r = base();
    r.praetor = Array.from({ length: 50 }, (_, i) => ({
      file: "x.rs",
      line: i,
      message: "m".repeat(100),
    }));
    const text = renderReport({ ...r, budgetBytes: 400 });
    expect(text.length).toBeLessThanOrEqual(420);
    expect(text).toContain("truncated");
  });
});
