// Tests for baseline classification + report rendering (the WIP contract).
import { describe, expect, it } from "vitest";
import { classifyGate, diagKey, renderReport, type GateObs, type VerifyResult } from "./baseline.ts";
import type { Gate } from "./gates.ts";

const gate: Gate = {
  id: ".::typecheck", dir: "/p", rel: ".", label: "typecheck",
  cmd: ["npm", "run", "typecheck"], tier: "cheap", parser: "tsc", auto: true,
};

function obs(diags: GateObs["diags"], status: GateObs["status"] = "fail"): GateObs {
  return { gateId: gate.id, status, diags, tookMs: 10, tail: "" };
}

const dFile = (file: string, msg: string, line: number | null = null) => ({ file, line, code: "TS1", message: msg });
const dGate = (msg: string) => ({ file: null, line: null, code: "gate", message: msg });

describe("diagKey", () => {
  it("stable across line drift; distinct per file/code/message", () => {
    expect(diagKey({ file: "/a.ts", line: 1, code: "TS1", message: "m" }))
      .toBe(diagKey({ file: "/a.ts", line: 99, code: "TS1", message: "m" }));
    expect(diagKey({ file: "/a.ts", line: 1, code: "TS1", message: "m" }))
      .not.toBe(diagKey({ file: "/b.ts", line: 1, code: "TS1", message: "m" }));
    expect(diagKey({ file: null, line: null, code: "gate", message: "x" }))
      .toBe(diagKey({ file: null, line: null, code: "gate", message: "x" }));
  });
});

describe("classifyGate", () => {
  it("first observation (no baseline) → everything pre-existing", () => {
    const out = classifyGate(gate, obs([dFile("/x.ts", "boom")]), undefined, new Set(), false);
    expect(out.map((c) => c.kind)).toEqual(["pre-existing"]);
  });

  it("key in baseline → pre-existing even when touched", () => {
    const out = classifyGate(gate, obs([dFile("/x.ts", "boom")]), obs([dFile("/x.ts", "boom", 5)]), new Set(["/x.ts"]), false);
    expect(out[0]!.kind).toBe("pre-existing");
  });

  it("new + touched file → in-progress (amber, I1)", () => {
    const base = obs([]);
    const out = classifyGate(gate, obs([dFile("/x.ts", "new")]), base, new Set(["/x.ts"]), false);
    expect(out[0]!.kind).toBe("in-progress");
  });

  it("new + untouched file → regression", () => {
    const base = obs([]);
    const out = classifyGate(gate, obs([dFile("/other.ts", "new")]), base, new Set(["/x.ts"]), false);
    expect(out[0]!.kind).toBe("regression");
  });

  it("new gate-level diagnostic → in-progress by default, regression under strict", () => {
    const base = obs([]);
    expect(classifyGate(gate, obs([dGate("npm test failed")]), base, new Set(), false)[0]!.kind).toBe("in-progress");
    expect(classifyGate(gate, obs([dGate("npm test failed")]), base, new Set(), true)[0]!.kind).toBe("regression");
  });

  it("strict upgrades touched in-progress → regression but keeps pre-existing", () => {
    const base = obs([dFile("/old.ts", "old")]);
    const cur = obs([dFile("/old.ts", "old"), dFile("/new.ts", "new")]);
    const out = classifyGate(gate, cur, base, new Set(["/new.ts"]), true);
    expect(out.find((c) => c.diag.message === "old")!.kind).toBe("pre-existing");
    expect(out.find((c) => c.diag.message === "new")!.kind).toBe("regression");
  });
});

function fakeResult(): VerifyResult {
  return {
    tier: "full",
    strict: false,
    at: 0,
    tookMs: 1000,
    outcomes: [
      { gate, obs: obs([], "ok"), baselinedHere: true },
      { gate: { ...gate, id: ".::test", label: "npm test", tier: "full" }, obs: obs([], "timeout"), baselinedHere: false },
    ],
    regressions: [{ gateId: gate.id, gateLabel: "typecheck", diag: dFile("/r.ts", "broke"), kind: "regression" }],
    inProgress: [{ gateId: gate.id, gateLabel: "typecheck", diag: dFile("/w.ts", "wip"), kind: "in-progress" }],
    preExisting: [
      { gateId: gate.id, gateLabel: "typecheck", diag: dFile("/p1.ts", "old1"), kind: "pre-existing" },
      { gateId: gate.id, gateLabel: "typecheck", diag: dFile("/p2.ts", "old2"), kind: "pre-existing" },
    ],
  };
}

describe("renderReport", () => {
  it("sections: header, gates, regressions, in-progress, pre-existing, not-to-verdict", () => {
    const lines = renderReport(fakeResult()).join("\n");
    expect(lines).toMatch(/VERIFY · full · 1\/2 gates green · 1 not green/);
    expect(lines).toMatch(/REGRESSIONS \(1\)/);
    expect(lines).toMatch(/IN-PROGRESS \(1\)/);
    expect(lines).toMatch(/PRE-EXISTING \(2\)/);
    expect(lines).toMatch(/npm test \(timeout\)/);
    expect(lines).toContain("/r.ts  [TS1] broke");
  });
  it("clean run says so", () => {
    const r = fakeResult();
    r.regressions = []; r.inProgress = []; r.preExisting = [];
    expect(renderReport(r).join("\n")).toContain("clean — no diagnostics");
  });
  it("caps detail lines at 18", () => {
    const r = fakeResult();
    r.preExisting = Array.from({ length: 25 }, (_, i) => ({
      gateId: gate.id, gateLabel: "typecheck", diag: dFile(`/f${i}.ts`, `m${i}`), kind: "pre-existing" as const,
    }));
    const lines = renderReport(r);
    expect(lines.some((l) => l.includes("… +7 more"))).toBe(true);
  });
});
