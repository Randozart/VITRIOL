import { describe, it, expect } from "vitest";
import { classifyTurn, recentErrorSignal, scratchpadContextLines, type ClassifyInput } from "./classifier.ts";

const base: ClassifyInput = {
  promptText: "fix the typo in README",
  recentErrorCount: 0,
  churnLoops: 0,
  scratchpadContextLines: [],
  filesTouched: 0,
  turnCount: 1,
};

describe("classifyTurn", () => {
  it("baseline turn is low complexity and safe", () => {
    const c = classifyTurn(base);
    expect(c.privacy).toBe("safe");
    expect(c.complexity).toBeLessThan(0.3);
  });

  it("churn loop is the strongest stuck signal", () => {
    const c = classifyTurn({ ...base, churnLoops: 1 });
    expect(c.signals.loop_detected).toBe(1);
    const plain = classifyTurn(base);
    expect(c.complexity - plain.complexity).toBeGreaterThanOrEqual(0.3 - 1e-9);
  });

  it("recent tool failures raise complexity", () => {
    const one = classifyTurn({ ...base, recentErrorCount: 1 });
    const two = classifyTurn({ ...base, recentErrorCount: 2 });
    const plain = classifyTurn(base);
    expect(one.complexity).toBeGreaterThan(plain.complexity);
    expect(two.complexity).toBeGreaterThan(one.complexity);
  });

  it("long prompts and deep-domain keywords compound", () => {
    const long = classifyTurn({ ...base, promptText: "x".repeat(2500) });
    const deep = classifyTurn({ ...base, promptText: "cuda kernel deadlock + systemd driver + dmesg segfault" });
    expect(long.complexity).toBeGreaterThan(0.3);
    expect(deep.signals.deep_domain_hits).toBeGreaterThanOrEqual(3);
    expect(deep.complexity).toBeGreaterThan(0.3);
  });

  it("scratchpad pressure adds complexity", () => {
    const lines = Array.from({ length: 12 }, (_, i) => `err line ${i}`);
    const c = classifyTurn({ ...base, scratchpadContextLines: lines });
    expect(c.complexity).toBeGreaterThan(classifyTurn(base).complexity);
  });

  it("secret paths and api keys are confidential", () => {
    expect(classifyTurn({ ...base, promptText: "read ~/.ssh/id_rsa and the .env file" }).privacy).toBe("confidential");
    expect(classifyTurn({ ...base, promptText: "my key is AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ12345" }).privacy).toBe("confidential");
    expect(classifyTurn({ ...base, promptText: "check credentials.json" }).privacy).toBe("confidential");
  });

  it("pii is sensitive but not confidential", () => {
    const c = classifyTurn({ ...base, promptText: "email the report to jane.doe@example.com please" });
    expect(c.privacy).toBe("sensitive");
  });

  it("dates in filenames are NOT pii (2026-09-02 field-test regression)", () => {
    const c = classifyTurn({ ...base, promptText: "continue the plan in docs/plans/2026-08-31-vault-rewrite-completion.md step 2" });
    expect(c.privacy).toBe("safe");
  });

  it("phone-like digit runs still count as sensitive", () => {
    const c = classifyTurn({ ...base, promptText: "call me at +49 170 555 1234 about the release" });
    expect(c.privacy).toBe("sensitive");
  });

  it("compile-error turns reach flagship territory (field-test regression)", () => {
    const c = classifyTurn({
      ...base,
      promptText: "continue the vault rewrite",
      recentErrorCount: 3,
      scratchpadContextLines: Array.from({ length: 12 }, (_, i) => `E0599 caller site ${i}`),
      filesTouched: 6,
      turnCount: 9,
    });
    // 0.15 base + 0.25 (errors≥2) + 0.10 (context>8) + 0.05 (files>5) = 0.55
    // → local-lg at t=0.5 (cutoff 0.65); cloud stays reserved for churn.
    expect(c.complexity).toBeCloseTo(0.55, 5);
    expect(c.complexity).toBeGreaterThanOrEqual(0.45);
    expect(c.complexity).toBeLessThan(0.65);
  });

  it("churn on top of compile errors crosses the cloud cutoff", () => {
    const c = classifyTurn({ ...base, recentErrorCount: 3, churnLoops: 1 });
    expect(c.complexity).toBeGreaterThan(0.65);
  });

  it("complexity never leaves [0,1]", () => {
    const worst = classifyTurn({
      promptText: "x".repeat(4000) + " cuda kernel systemd driver deadlock segfault",
      recentErrorCount: 9,
      churnLoops: 4,
      scratchpadContextLines: Array.from({ length: 40 }, (_, i) => `l${i}`),
      filesTouched: 30,
      turnCount: 200,
    });
    expect(worst.complexity).toBeLessThanOrEqual(1);
    expect(worst.complexity).toBeGreaterThan(0.8);
  });
});

describe("recentErrorSignal", () => {
  it("counts isError results", () => {
    expect(recentErrorSignal([{ isError: true }])).toBe(1);
  });
  it("counts compile errors hidden in successful bash output (field-test regression)", () => {
    const cargo = { isError: false, text: "error[E0599]: no method named `put_meta`\nerror: could not compile `ontic` due to 30 previous errors" };
    expect(recentErrorSignal([cargo])).toBeGreaterThanOrEqual(1);
  });
  it("does not count clean output", () => {
    expect(recentErrorSignal([{ isError: false, text: "warning: unused import\n    Finished dev profile" }])).toBe(0);
  });
  it("respects the window", () => {
    const old = Array.from({ length: 12 }, () => ({ isError: false, text: "error[E0308]: mismatched types" }));
    const clean = Array.from({ length: 6 }, () => ({ isError: false, text: "all good" }));
    expect(recentErrorSignal([...old, ...clean])).toBe(0);
  });
});

describe("scratchpadContextLines", () => {
  it("parses the file-format h2 section", () => {
    const md = "# Scratchpad\n\n## facts\n- a\n\n## context\n- E0599 at line 142\n- E0432 at line 8\n\n## leads\n- try X\n";
    expect(scratchpadContextLines(md)).toEqual(["E0599 at line 142", "E0432 at line 8"]);
  });

  it("parses the rendered h3 section", () => {
    const md = "## Scratchpad\n[3/120 lines]\n### facts\n- a\n### context\n- b\n### leads\n- c\n";
    expect(scratchpadContextLines(md)).toEqual(["b"]);
  });

  it("returns empty when no context section", () => {
    expect(scratchpadContextLines("## facts\n- a\n")).toEqual([]);
  });
});
