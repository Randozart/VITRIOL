import { describe, it, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkFor, diagConfig, renderDiagnostics, tailCap, type FileDiag } from "./diag.ts";

describe("selected checkers actually run on this host", () => {
  it("py_compile exits 0 on valid syntax and non-zero on broken", () => {
    const dir = mkdtempSync(join(tmpdir(), "tris-diag-"));
    try {
      const good = join(dir, "good.py");
      const bad = join(dir, "bad.py");
      writeFileSync(good, "x = 1\n");
      writeFileSync(bad, "def (:\n");
      const run = (f: string): boolean => {
        const cmd = checkFor(f)!;
        try {
          execFileSync(cmd.argv[0], cmd.argv.slice(1), { stdio: "pipe" });
          return true;
        } catch {
          return false;
        }
      };
      expect(run(good)).toBe(true);
      expect(run(bad)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

const d = (file: string, ok: boolean, output = ""): FileDiag => ({ file, ok, output });

describe("diagConfig", () => {
  it("on by default, kill switch off; praetor opt-in", () => {
    expect(diagConfig({}).enabled).toBe(true);
    expect(diagConfig({ TRIS_NO_DIAGNOSTICS: "1" }).enabled).toBe(false);
    expect(diagConfig({}).praetor).toBe(false);
    expect(diagConfig({ TRIS_DIAG_PRAETOR: "1" }).praetor).toBe(true);
  });

  it("budget matches §3.3 (~300 tokens at 3.5 chars/token)", () => {
    expect(Math.round(diagConfig({}).budgetChars / 3.5)).toBeLessThanOrEqual(300);
  });
});

describe("checkFor", () => {
  it("selects file-local fast checks by extension", () => {
    expect(checkFor("src/app.py")!.argv).toEqual(["python3", "-m", "py_compile", "src/app.py"]);
    expect(checkFor("run.sh")!.argv[0]).toBe("bash");
    expect(checkFor("bin/tool.mjs")!.argv[0]).toBe("node");
    expect(checkFor("cfg.json")!.argv[0]).toBe("node");
  });

  it("returns null for unknown extensions (no check, no noise)", () => {
    expect(checkFor("README.md")).toBeNull();
    expect(checkFor("model.gguf")).toBeNull();
  });

  it("routes source files to praetor only when opted in", () => {
    expect(checkFor("main.rs", diagConfig({}))).toBeNull();
    expect(checkFor("main.rs", diagConfig({ TRIS_DIAG_PRAETOR: "1" }))!.argv[0]).toBe("praetor-diag");
  });
});

describe("renderDiagnostics", () => {
  it("stays silent on clean checks (zero token cost)", () => {
    expect(renderDiagnostics([d("a.py", true)], 1050)).toBe("");
  });

  it("names failing files and counts them", () => {
    const out = renderDiagnostics([d("a.py", false, "SyntaxError: bad"), d("b.py", true)], 1050);
    expect(out).toContain("1 file(s) failed");
    expect(out).toContain("--- a.py ---");
    expect(out).toContain("SyntaxError");
    expect(out).not.toContain("--- b.py ---");
  });

  it("caps at budget with a truncation note", () => {
    const fat = Array.from({ length: 8 }, (_, i) => d(`f${i}.py`, false, "e".repeat(400)));
    const out = renderDiagnostics(fat, 1050);
    expect(out.length).toBeLessThanOrEqual(1050 + 80); // note overhead only
    expect(out).toContain("truncated to budget");
  });
});

describe("tailCap", () => {
  it("keeps the tail (errors live at the end) and marks a cut", () => {
    const t = tailCap("x".repeat(1000) + "BOOM", 100);
    expect(t.length).toBeLessThanOrEqual(101);
    expect(t).toContain("BOOM");
    expect(t.startsWith("…")).toBe(true);
  });

  it("passes short output through", () => {
    expect(tailCap("short", 100)).toBe("short");
  });
});
