// Prediction rules (codex-qol stage 8, I7: advisory-only).
import { describe, expect, it } from "vitest";
import { dirtyCount, predict, readVerifyLast, type VerifyLast } from "./predict.ts";

const v = (over: Partial<VerifyLast> = {}): VerifyLast => ({
  version: 1,
  ts: 1_700_000_000_000,
  tier: "cheap",
  ok: true,
  regressions: [],
  in_progress: [],
  ...over,
});

describe("predict", () => {
  it("follows the task in progress, with process state only as an annotation", () => {
    const line = predict(v({ ok: false, regressions: ["cargo-test"] }), 3, 0, "wire the Esc handler");
    expect(line).toBe(
      "▸ next: wire the Esc handler · 1 gate failing · 3 changed files to commit",
    );
    expect(predict(v(), 0, 0, "wire the Esc handler")).toBe("▸ next: wire the Esc handler");
  });

  it("clips a task line and never turns it into a paragraph", () => {
    const long = predict(v(), 0, 0, `${"x".repeat(200)}.`);
    expect(long!.length).toBeLessThanOrEqual(140);
    expect(long!.endsWith("…")).toBe(true);
    expect(predict(v(), 0, 0, "  finish the audit.  ")).toBe("▸ next: finish the audit");
  });

  it("stays silent without a task — harness bookkeeping is not advice (D4)", () => {
    expect(predict(v({ ok: false, regressions: ["x"] }), 3, 0)).toBeNull();
    expect(predict(null, 5, 0)).toBeNull();
    expect(predict(v(), 1, 0)).toBeNull();
    expect(predict(v(), 0, 0, "   ")).toBeNull();
  });

  it("stays silent when everything is green and clean", () => {
    expect(predict(v(), 0, 0)).toBeNull();
    expect(predict(null, 0, 0)).toBeNull();
  });
});

describe("readVerifyLast", () => {
  it("tolerates missing and corrupt files", () => {
    expect(readVerifyLast("/definitely/not/here")).toBeNull();
    expect(readVerifyLast(".", () => "{corrupt")).toBeNull();
    expect(readVerifyLast(".", () => JSON.stringify(v()))).toEqual(v());
  });
});

describe("dirtyCount", () => {
  it("counts porcelain lines; empty output means git failure", () => {
    expect(dirtyCount(() => " M a\n?? b\n")).toBe(2);
    expect(dirtyCount(() => "")).toBe(-1);
  });
});
