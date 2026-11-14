import { describe, expect, it } from "vitest";
import type { VerifyResult } from "../verify/baseline.ts";
import {
  failingGates,
  renderVerifyTail,
  shouldRun,
  signatureOf,
  verifyOnEditConfig,
} from "./rules.ts";

function changed(entries: Array<[string, string]>): Map<string, string> {
  return new Map(entries);
}

describe("shouldRun", () => {
  it("does nothing when no file was edited", () => {
    const d = shouldRun({ changed: changed([]), lastSignature: null, lastRunAt: 0, now: 1000, cooldownMs: 0 });
    expect(d.run).toBe(false);
  });

  it("skips an unchanged file set (I4)", () => {
    const files = changed([["/a.ts", "h1"]]);
    const sig = signatureOf(files);
    const d = shouldRun({ changed: files, lastSignature: sig, lastRunAt: 0, now: 100_000, cooldownMs: 0 });
    expect(d.run).toBe(false);
    expect(d.reason).toContain("same");
  });

  it("runs when a hash changed", () => {
    const files = changed([["/a.ts", "h2"]]);
    const d = shouldRun({
      changed: files,
      lastSignature: signatureOf(changed([["/a.ts", "h1"]])),
      lastRunAt: 0,
      now: 100_000,
      cooldownMs: 10_000,
    });
    expect(d.run).toBe(true);
  });

  it("respects the cooldown", () => {
    const files = changed([["/b.ts", "h"]]);
    const d = shouldRun({ changed: files, lastSignature: null, lastRunAt: 95_000, now: 100_000, cooldownMs: 10_000 });
    expect(d.run).toBe(false);
    expect(d.reason).toBe("cooldown");
  });
});

describe("config", () => {
  it("defaults on with sane budgets and an env kill switch", () => {
    const cfg = verifyOnEditConfig({} as NodeJS.ProcessEnv);
    expect(cfg.enabled).toBe(true);
    expect(cfg.budgetMs).toBe(25_000);
    expect(verifyOnEditConfig({ OFFICINA_VERIFY_ON_EDIT: "0" } as NodeJS.ProcessEnv).enabled).toBe(
      false,
    );
    expect(
      verifyOnEditConfig({ OFFICINA_VERIFY_BUDGET_MS: "999999" } as NodeJS.ProcessEnv).budgetMs,
    ).toBe(300_000);
  });
});

describe("failure rendering", () => {
  it("flattens regressions + in-progress and renders a bounded tail", () => {
    const result = {
      regressions: [
        { gateId: "tsc", gateLabel: "typecheck", diag: { message: "TS2304: missing name" } },
      ],
      inProgress: [{ gateId: "cargo-check", gateLabel: "cargo check", diag: { message: "E0308" } }],
      preExisting: [{ gateId: "old", gateLabel: "old", diag: { message: "pre-existing" } }],
    } as unknown as VerifyResult;
    const failures = failingGates(result);
    expect(failures).toHaveLength(2);
    expect(failures[0]!.id).toBe("tsc");
    const tail = renderVerifyTail(failures, 1_200);
    expect(tail).toContain("2 gate(s) failing");
    expect(tail).toContain("TS2304");
    expect(tail).not.toContain("pre-existing");
  });

  it("is empty when green (zero tokens)", () => {
    expect(renderVerifyTail([], 1_200)).toBe("");
  });

  it("truncates within the budget", () => {
    const failures = Array.from({ length: 40 }, (_, i) => ({
      id: `gate-${i}`,
      label: "gate",
      message: "x".repeat(80),
    }));
    const tail = renderVerifyTail(failures, 400);
    expect(tail.length).toBeLessThanOrEqual(400);
    expect(tail).toContain("truncated");
  });
});
