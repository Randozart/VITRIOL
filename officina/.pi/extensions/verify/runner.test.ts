// Tests for the run orchestration (fake exec — no real gates in unit tests).
import { describe, expect, it } from "vitest";
import type { GateObs } from "./baseline.ts";
import type { Gate } from "./gates.ts";
import { execGate, normalizeTouched, parseVerifyArgs, runVerify, type ExecFn } from "./runner.ts";

const root: Gate = {
  id: ".::typecheck", dir: "/p", rel: ".", label: "typecheck",
  cmd: ["npm", "run", "typecheck"], tier: "cheap", parser: "tsc", auto: true,
};
const testGate: Gate = { ...root, id: ".::test", label: "npm test", tier: "full", parser: "vitest", cmd: ["npm", "test"] };

/** scriptable exec: responses keyed by label, in call order */
function scripted(responses: Record<string, string[] | { err: string }>): { fn: ExecFn; calls: string[][] } {
  const calls: string[][] = [];
  const fn: ExecFn = async (argv, cwd) => {
    calls.push([...argv]);
    const key = Object.keys(responses).find((k) => argv.join(" ").includes(k)) ?? argv.join(" ");
    const seq = responses[key];
    if (!seq || !Array.isArray(seq)) return { code: 127, out: "", spawnErr: `${key}: command not found` };
    const out = seq.length > 1 ? seq.shift()! : seq[0]!;
    return out === "" ? { code: 0, out: "" } : { code: 1, out };
  };
  return { fn, calls };
}

const BASELINE_ERR = "/p/old.ts(1,1): error TS1: preexisting";
const NEW_ERR_TOUCHED = "/p/touched.ts(2,2): error TS2: mine";
const NEW_ERR_UNTOUCHED = "/p/bystander.ts(3,3): error TS3: broke";

describe("runVerify", () => {
  it("first observation = baseline; second run diffs against it (I1 ladder)", async () => {
    const baseline = new Map<string, GateObs>();
    const ex = scripted({
      typecheck: [BASELINE_ERR, BASELINE_ERR + "\n" + NEW_ERR_TOUCHED + "\n" + NEW_ERR_UNTOUCHED],
    });

    // run 1: session-start — the going-in reality
    const r1 = await runVerify([root], { tier: "cheap", strict: false, baseline, touched: new Set(), exec: ex.fn });
    expect(r1.preExisting).toHaveLength(1);
    expect(r1.regressions).toHaveLength(0);
    expect(r1.inProgress).toHaveLength(0);

    // run 2: edited touched.ts; bystander broke anyway
    const r2 = await runVerify([root], {
      tier: "cheap", strict: false, baseline,
      touched: new Set(["/p/touched.ts"]), exec: ex.fn,
    });
    expect(r2.preExisting).toHaveLength(1);
    expect(r2.inProgress.map((c) => c.diag.file)).toEqual(["/p/touched.ts"]);
    expect(r2.regressions.map((c) => c.diag.file)).toEqual(["/p/bystander.ts"]);
  });

  it("gate not yet baselined → first failures are pre-existing, not red", async () => {
    const baseline = new Map<string, GateObs>();
    const ex = scripted({ "npm test": ["FAIL extensions/x.test.ts"] });
    const r = await runVerify([testGate], { tier: "full", strict: false, baseline, touched: new Set(), exec: ex.fn });
    expect(r.regressions).toHaveLength(0);
    expect(r.preExisting).toHaveLength(1);
    expect(baseline.get(testGate.id)!.diags).toHaveLength(1);
  });

  it("failing gate with unparseable output → gate-level diagnostic", async () => {
    const baseline = new Map<string, GateObs>();
    baseline.set(testGate.id, { gateId: testGate.id, status: "ok", diags: [], tookMs: 1, tail: "" });
    const ex = scripted({ "npm test": ["some weird stack\nAssertionError"] });
    const r = await runVerify([testGate], { tier: "full", strict: false, baseline, touched: new Set(), exec: ex.fn });
    // new gate-level diagnostic defaults to in-progress (not red), I1
    expect(r.inProgress[0]!.diag).toMatchObject({ file: null, code: "gate" });
    const strictRun = await runVerify([testGate], { tier: "full", strict: true, baseline, touched: new Set(), exec: ex.fn });
    expect(strictRun.regressions[0]!.diag).toMatchObject({ file: null, code: "gate" });
  });

  it("cheap tier skips full gates; full tier runs both", async () => {
    const ex = scripted({ typecheck: [""], "npm test": [""] });
    const b1 = new Map<string, GateObs>();
    await runVerify([root, testGate], { tier: "cheap", strict: false, baseline: b1, touched: new Set(), exec: ex.fn });
    expect(ex.calls).toHaveLength(1);
    const ex2 = scripted({ typecheck: [""], "npm test": [""] });
    await runVerify([root, testGate], { tier: "full", strict: false, baseline: new Map(), touched: new Set(), exec: ex2.fn });
    expect(ex2.calls).toHaveLength(2);
  });

  it("budget defers remaining gates (status deferred, no baseline entry)", async () => {
    const baseline = new Map<string, GateObs>();
    const ex = scripted({ typecheck: [""], "npm test": [""] });
    const r = await runVerify([root, testGate], {
      tier: "full", strict: false, baseline, touched: new Set(), exec: ex.fn, budgetMs: -1,
    });
    expect(r.outcomes.every((o) => o.obs.status === "deferred")).toBe(true);
    expect(baseline.size).toBe(0);
  });

  it("stale() stops the run BEFORE touching the fresh baseline", async () => {
    const baseline = new Map<string, GateObs>();
    let stale = false;
    const ex = scripted({ typecheck: [BASELINE_ERR] });
    const r = await runVerify([root], {
      tier: "cheap", strict: false, baseline, touched: new Set(), exec: ex.fn,
      stale: () => stale,
    });
    // stale BEFORE first gate → nothing ran, nothing recorded
    stale = true;
    const r2 = await runVerify([root], {
      tier: "cheap", strict: false, baseline, touched: new Set(), exec: ex.fn, stale: () => stale,
    });
    expect(r.outcomes).toHaveLength(1);
    expect(r2.outcomes).toHaveLength(0);
    expect(baseline.size).toBe(1); // only run 1's observation survived
  });

  it("spawn error → status error, no diagnostics, never red", async () => {
    const baseline = new Map<string, GateObs>();
    baseline.set(root.id, { gateId: root.id, status: "ok", diags: [], tookMs: 1, tail: "" });
    const ex: ExecFn = async () => ({ code: 127, out: "", spawnErr: "ruff: command not found" });
    const r = await runVerify([root], { tier: "cheap", strict: false, baseline, touched: new Set(), exec: ex });
    expect(r.outcomes[0]!.obs.status).toBe("error");
    expect(r.regressions).toHaveLength(0);
    expect(r.inProgress).toHaveLength(0);
  });

  it("timeout → status timeout, diagnostics dropped (no verdict)", async () => {
    const baseline = new Map<string, GateObs>();
    baseline.set(root.id, { gateId: root.id, status: "ok", diags: [], tookMs: 1, tail: "" });
    const ex: ExecFn = async () => ({ code: 124, out: BASELINE_ERR, timedOut: true });
    const r = await runVerify([root], { tier: "cheap", strict: false, baseline, touched: new Set(), exec: ex });
    expect(r.outcomes[0]!.obs.status).toBe("timeout");
    expect(r.preExisting).toHaveLength(0);
  });

  it("onGate fires per gate with running-progress semantics", async () => {
    const seen: string[] = [];
    const ex = scripted({ typecheck: [""], "npm test": [""] });
    await runVerify([root, testGate], {
      tier: "full", strict: false, baseline: new Map(), touched: new Set(), exec: ex.fn,
      onGate: (g) => seen.push(g.id),
    });
    expect(seen).toEqual([".::typecheck", ".::test"]);
  });
});

describe("parseVerifyArgs", () => {
  it("defaults full, non-strict", () => expect(parseVerifyArgs("")).toEqual({ tier: "full", strict: false }));
  it("cheap | strict | --strict", () => {
    expect(parseVerifyArgs("cheap")).toEqual({ tier: "cheap", strict: false });
    expect(parseVerifyArgs("strict")).toEqual({ tier: "full", strict: true });
    expect(parseVerifyArgs("cheap --strict")).toEqual({ tier: "cheap", strict: true });
  });
});

describe("normalizeTouched", () => {
  it("absolutizes relative paths, keeps absolute", () => {
    expect(normalizeTouched("src/a.ts", "/p")).toBe("/p/src/a.ts");
    expect(normalizeTouched("/p/src/a.ts", "/other")).toBe("/p/src/a.ts");
  });
});

describe("execGate", () => {
  it("ENOENT → spawnErr (never throws)", async () => {
    const r = await execGate(["definitely-not-a-real-binary-xyz"], process.cwd(), 5000);
    expect(r.spawnErr).toMatch(/command not found/);
    expect(r.code).toBe(127);
  });
  it("successful command → code 0 with output", async () => {
    const r = await execGate(["true"], process.cwd(), 5000);
    expect(r.code).toBe(0);
  });
});
