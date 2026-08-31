import { describe, it, expect } from "vitest";
import { exitCodeNote, isRtkTarget, rawPathFor, renderSummary, rtkConfig, summarize } from "./rtk.ts";

describe("rtkConfig", () => {
  it("defaults on empty env", () => {
    const cfg = rtkConfig({});
    expect(cfg.enabled).toBe(true);
    expect(cfg.thresholdChars).toBe(800);
    expect(cfg.tailLines).toBe(20);
    expect(cfg.rawDir).toBe(".pi/rtk");
  });

  it("kill switch", () => {
    expect(rtkConfig({ TRIS_NO_RTK_OUTPUT: "1" }).enabled).toBe(false);
  });

  it("bad numbers fall back to defaults", () => {
    expect(rtkConfig({ TRIS_RTK_TAIL: "many" }).tailLines).toBe(20);
    expect(rtkConfig({ TRIS_RTK_THRESHOLD: "0" }).thresholdChars).toBe(800);
  });
});

describe("isRtkTarget", () => {
  it("matches test/build/install commands", () => {
    for (const c of ["npm test", "pytest plugins/ -q", "cargo build --release", "make -j8", "tsc --noEmit", "pip install -r req.txt", "npm run typecheck"]) {
      expect(isRtkTarget(c)).toBe(true);
    }
  });

  it("leaves deliberate text commands alone", () => {
    for (const c of ["git status", "ls -la", "cat README.md", "echo hi", "grep -r foo src/"]) {
      expect(isRtkTarget(c)).toBe(false);
    }
  });
});

describe("summarize", () => {
  const noise = (n: number, filler = "  File compiled successfully module_" + n) =>
    Array.from({ length: n }, (_, i) => filler + i).join("\n");
  const failing = [
    "running 3 tests",
    "test a ... ok",
    "test b ... FAILED",
    "error: assertion failed: expected 2, got 3",
    "src/main.rs:42:5: error[E0308]: mismatched types",
    noise(50),
    "failures:",
    "    test_b",
  ].join("\n");

  it("keeps error lines and counts them", () => {
    const s = summarize("cargo test", failing, rtkConfig({}));
    expect(s.errorLines.join("\n")).toContain("mismatched types");
    expect(s.errorLines.join("\n")).toContain("FAILED");
    expect(s.counts.errorLines).toBeGreaterThan(0);
  });

  it("achieves the 60-90% reduction target on fat output", () => {
    const big = noise(400) + "\n" + "error: boom\n";
    const s = summarize("make", big, rtkConfig({}));
    const ratio = s.savedChars / big.length;
    expect(ratio).toBeGreaterThanOrEqual(0.6);
  });

  it("reports the exit code when pi embeds it", () => {
    const note = exitCodeNote("npm test", "output…\nProcess exited with code 1");
    expect(note).toContain("exit 1");
    expect(note).toContain("$ npm test");
  });
});

describe("renderSummary", () => {
  it("references the raw log path so output is recoverable", () => {
    const s = summarize("pytest", "x".repeat(5000) + "\nFAILED test_one\n", rtkConfig({}));
    const out = renderSummary(s, ".pi/rtk/tc1.log");
    expect(out).toContain(".pi/rtk/tc1.log");
    expect(out).toContain("FAILED test_one");
    expect(out).toContain("exit unknown");
  });
});

describe("rawPathFor", () => {
  it("sanitizes tool call ids and appends .log (no traversal: slashes cannot survive)", () => {
    const p = rawPathFor(".pi/rtk", "../../etc/passwd");
    expect(p).toBe(".pi/rtk/.._.._etc_passwd.log");
    expect(p.split("/")).toHaveLength(3); // dir + file only
    expect(rawPathFor(".pi/rtk", "call-9_f.1")).toBe(".pi/rtk/call-9_f.1.log");
  });
});
