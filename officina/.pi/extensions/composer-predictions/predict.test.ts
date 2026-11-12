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
  it("points at failing gates first", () => {
    const line = predict(v({ ok: false, regressions: ["cargo-test"] }), 3, 0);
    expect(line).toContain("1 gate failing");
    expect(line).toContain("/verify full"); // cheap run → suggest the full one
    expect(predict(v({ ok: false, tier: "full", in_progress: ["a", "b"] }), 0, 0))
      .toContain("2 gates failing");
    expect(predict(v({ ok: false, tier: "full", in_progress: ["a", "b"] }), 0, 0))
      .toContain("(/verify)");
  });

  it("suggests a commit when gates are green and the tree is dirty", () => {
    expect(predict(v(), 2, 0)).toBe("▸ next: commit the work (2 changed files)");
    expect(predict(v(), 1, 0)).toBe("▸ next: commit the work (1 changed file)");
  });

  it("stays silent when everything is green and clean", () => {
    expect(predict(v(), 0, 0)).toBeNull();
    expect(predict(null, 0, 0)).toBeNull();
  });

  it("still nudges a commit with no verify data", () => {
    expect(predict(null, 5, 0)).toContain("5 changed files");
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
