// Tests for verify widget line rendering (ANSI surfaces).
import { describe, expect, it } from "vitest";
import { renderWidgetLines } from "./widget.ts";
import type { VerifyResult } from "./baseline.ts";
import type { Gate } from "./gates.ts";

const gate: Gate = {
  id: ".::typecheck", dir: "/p", rel: ".", label: "typecheck",
  cmd: ["x"], tier: "cheap", parser: "tsc", auto: true,
};

function res(partial: Partial<VerifyResult>): VerifyResult {
  return {
    tier: "cheap", strict: false, at: 0, tookMs: 1,
    outcomes: [], regressions: [], inProgress: [], preExisting: [],
    ...partial,
  };
}

describe("renderWidgetLines", () => {
  it("hidden with no result and nothing running", () => {
    expect(renderWidgetLines(null, [])).toBeUndefined();
  });

  it("running shows spinner + labels", () => {
    const lines = renderWidgetLines(null, ["typecheck", "cargo check"])!;
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain("⏳");
    expect(lines[0]).toContain("typecheck");
    expect(lines[0]).toContain("cargo check");
  });

  it("all green renders safety header", () => {
    const lines = renderWidgetLines(res({ outcomes: [{ gate, obs: { gateId: gate.id, status: "ok", diags: [], tookMs: 5, tail: "" }, baselinedHere: true }] }), [])!;
    expect(lines[0]).toContain("✓ 1");
    expect(lines[0]).toContain("\x1b[38;2;"); // ANSI truecolor
  });

  it("regressions switch header to substrate red", () => {
    const lines = renderWidgetLines(
      res({
        outcomes: [{ gate, obs: { gateId: gate.id, status: "fail", diags: [], tookMs: 5, tail: "" }, baselinedHere: true }],
        regressions: [{ gateId: gate.id, gateLabel: "typecheck", diag: { file: "/x.ts", line: 1, code: "TS1", message: "m" }, kind: "regression" }],
      }),
      [],
    )!;
    expect(lines[0]).toContain("✗ 1 reg");
    expect(lines[0]).toContain("\x1b[38;2;248;81;73m"); // substrate #f85149
  });

  it("timeout gates surface as ⏱", () => {
    const lines = renderWidgetLines(
      res({ outcomes: [{ gate, obs: { gateId: gate.id, status: "timeout", diags: [], tookMs: 5, tail: "" }, baselinedHere: true }] }),
      [],
    )!;
    expect(lines[0]).toContain("⏱ 1");
  });
});
