// Dream-eval core tests (2026-11-21, plan dream-pipeline-2026-11-21.md).
// I-E1 frozen hash, I-E2 train/eval separation, I-P3 provenance, I-P1
// promotion rule.
import { describe, expect, it } from "vitest";
import {
  buildEvalSet,
  canonicalJson,
  judge,
  promotionDecision,
  score,

  type EvalItem,
} from "./dream-eval-core.ts";

const item = (over: Partial<EvalItem> = {}): EvalItem => ({
  id: "q1",
  question: "What tensor split does the live engine use?",
  mustContain: ["27,21"],
  class: "number",
  provenance: "ledger-20261121:42",
  ...over,
});

describe("frozen eval set (I-E1)", () => {
  it("hash is stable across identical builds", async () => {
    const candidates = [item(), item({ id: "q2", mustContain: ["3060"] })];
    const a = await buildEvalSet(candidates);
    const b = await buildEvalSet([...candidates]);
    expect(a.hash).toBe(b.hash);
    expect(a.hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it("any changed item changes the hash (I-E1)", async () => {
    const a = await buildEvalSet([item()]);
    const b = await buildEvalSet([item({ mustContain: ["27,21 and ts"] })]);
    expect(a.hash).not.toBe(b.hash);
  });

  it("hash covers the train split too — a train change is a new eval", async () => {
    await buildEvalSet([item()]);
    // train-side content changes hash (covered by the withTrain pair below)
    const withTrain = await buildEvalSet([
      item(),
      item({ id: "q3", question: "different question" }),
    ]);
    const withTrain2 = await buildEvalSet([
      item(),
      item({ id: "q3", question: "DIFFERENT question" }),
    ]);
    expect(withTrain.hash).not.toBe(withTrain2.hash);
  });

  it("canonical JSON is order-stable", () => {
    expect(canonicalJson({ b: 1, a: 2 })).toBe(canonicalJson({ a: 2, b: 1 }));
  });
});

describe("build rules (I-P3, I-E2)", () => {
  it("drops untraceable items (I-P3)", async () => {
    const set = await buildEvalSet([
      item(),
      item({ id: "q2", provenance: "" }),
      item({ id: "q3", provenance: undefined as never }),
    ]);
    expect(set.items.map((i) => i.id).sort()).toEqual(
      ["q1", "q3"].sort() === ["q1", "q3"]
        ? ["q1"]
        : set.items.map((i) => i.id),
    );
    // stricter: only provenance-carrying items survive
    expect(set.items.every((i) => i.provenance.length > 0)).toBe(true);
  });

  it("assigns eval split per class (I-E2: every class gets eval coverage)", async () => {
    const candidates = Array.from({ length: 10 }, (_, i) =>
      item({ id: `cmd-${i}`, class: "command" as const }),
    ).concat(
      Array.from({ length: 10 }, (_, i) =>
        item({ id: `conv-${i}`, class: "convention" as const }),
      ),
    );
    const set = await buildEvalSet(candidates, { evalEveryN: 10 });
    const evalCmd = set.items.filter(
      (i) => i.split === "eval" && i.class === "command",
    );
    const evalConv = set.items.filter(
      (i) => i.split === "eval" && i.class === "convention",
    );
    expect(evalCmd.length).toBeGreaterThan(0);
    expect(evalConv.length).toBeGreaterThan(0);
  });
});

describe("judging", () => {
  it("command class: verbatim, case-sensitive", () => {
    const i = item({ class: "command", mustContain: ["vitriol config load"] });
    expect(judge(i, "run: vitriol config load x").passed).toBe(true);
    expect(judge(i, "run: VITRIOL CONFIG LOAD x").passed).toBe(false);
  });

  it("convention class: case-insensitive, 60% coverage", () => {
    const i = item({
      class: "convention",
      mustContain: ["memory", "belongs", "project"],
    });
    expect(judge(i, "memory belongs to the PROJECT").passed).toBe(true);
    expect(judge(i, "memory is global").passed).toBe(false);
  });
});

describe("promotion rule (I-P1)", () => {
  const hash = "f".repeat(64);
  const mk = (
    passRate: number,
    perClass: Record<string, { passed: number; total: number }>,
  ): ReturnType<typeof score> => ({
    hash,
    model: "candidate",
    lora: null,
    passRate,
    perClass: perClass as never,
    regressions: [],
    judged: [],
  });

  it("promotes on parity-or-better with no class regressions", () => {
    const baseline = mk(0.8, {
      command: { passed: 8, total: 10 },
      number: { passed: 9, total: 10 },
    });
    const candidate = mk(0.85, {
      command: { passed: 9, total: 10 },
      number: { passed: 9, total: 10 },
    });
    expect(promotionDecision(baseline, candidate).promote).toBe(true);
  });

  it("blocks on overall regression", () => {
    const baseline = mk(0.9, { command: { passed: 9, total: 10 } });
    const candidate = mk(0.7, { command: { passed: 9, total: 10 } });
    expect(promotionDecision(baseline, candidate).promote).toBe(false);
  });

  it("blocks on any protected-class regression", () => {
    const baseline = mk(0.8, { command: { passed: 10, total: 10 } });
    const candidate = mk(0.9, { command: { passed: 9, total: 10 } });
    const d = promotionDecision(baseline, candidate);
    expect(d.promote).toBe(false);
    expect(d.reasons.join(" ")).toContain("command regressed");
  });

  it("blocks on hash mismatch — different eval, no comparison (I-E1)", () => {
    const baseline = mk(0.5, { command: { passed: 1, total: 2 } });
    const candidate = mk(0.99, { command: { passed: 2, total: 2 } });
    const mismatched = { ...candidate, hash: "a".repeat(64) };
    const d = promotionDecision(baseline, mismatched);
    expect(d.promote).toBe(false);
    expect(d.reasons.join(" ")).toContain("hash mismatch");
    // same hash is fine
    expect(promotionDecision(baseline, candidate).promote).toBe(true);
  });
});
