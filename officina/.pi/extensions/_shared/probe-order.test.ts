// Probe-order tests (2026-11-21, plan free-inference-probe-routing S1).
// I1 parity, I2 ok-first, I3 context_limit usability.
import { describe, expect, it } from "vitest";
import { orderCandidates, seedFallbackList } from "./probe-order.ts";
import type { LaneKeyed } from "./probe-order.ts";

interface TestLane {
  id: string;
  limitTokens?: number;
}

const KEYS: LaneKeyed<TestLane> = {
  laneId: (l) => l.id,
  limitTokens: (l) => l.limitTokens,
};

const lane = (id: string, limitTokens?: number): TestLane => ({
  id,
  ...(limitTokens !== undefined ? { limitTokens } : {}),
});

describe("orderCandidates (I1)", () => {
  it("unprobed pool keeps catalog order", () => {
    const input = [lane("a/x"), lane("b/y"), lane("c/z")];
    expect(orderCandidates(input, {}, KEYS)).toEqual(input);
    expect(orderCandidates(input, {}, KEYS)).toEqual(input);
  });
});

describe("orderCandidates (I2)", () => {
  it("probe-ok lanes lead, fastest first", () => {
    const input = [lane("slow/ok"), lane("dead/lane"), lane("fast/ok")];
    const probes = {
      "slow/ok": { kind: "ok", latencyMs: 9000, failCount: 0, nextProbeAt: 0 },
      "fast/ok": { kind: "ok", latencyMs: 2000, failCount: 0, nextProbeAt: 0 },
      "dead/lane": { kind: "probe_error", failCount: 3, nextProbeAt: 0 },
    };
    const out = orderCandidates(input, probes, KEYS);
    expect(out.map((c) => c.id)).toEqual(["fast/ok", "slow/ok", "dead/lane"]);
  });

  it("every ok lane precedes every failing lane (I2)", () => {
    const input = [lane("f/1"), lane("ok/2"), lane("f/3"), lane("ok/4")];
    const probes = {
      "ok/2": { kind: "ok", latencyMs: 5000, failCount: 0, nextProbeAt: 0 },
      "ok/4": { kind: "ok", latencyMs: 6000, failCount: 0, nextProbeAt: 0 },
      "f/1": { kind: "cooling", failCount: 1, nextProbeAt: 0 },
      "f/3": { kind: "invalid", failCount: 2, nextProbeAt: 0 },
    };
    const out = orderCandidates(input, probes, KEYS).map((c) => c.id);
    expect(out.slice(0, 2).sort()).toEqual(["ok/2", "ok/4"]);
    expect(out.slice(2).sort()).toEqual(["f/1", "f/3"]);
  });
});

describe("orderCandidates (I3)", () => {
  it("context-limited lane stays eligible when it fits", () => {
    const input = [lane("big/ctx", 200_000), lane("plain")];
    const probes = {
      "big/ctx": { kind: "context_limit", failCount: 1, nextProbeAt: 0 },
    };
    const out = orderCandidates(input, probes, KEYS);
    // Unprobed "plain" still leads (bucket 1 vs 2) but the lane is NOT dead.
    expect(out.map((c) => c.id)).toEqual(["plain", "big/ctx"]);
  });

  it("context_limit without a learned limit ranks as failing", () => {
    const input = [lane("unknown/ctx"), lane("dead")];
    const probes = {
      "unknown/ctx": { kind: "context_limit", failCount: 1, nextProbeAt: 0 },
      dead: { kind: "probe_error", failCount: 5, nextProbeAt: 0 },
    };
    const out = orderCandidates(input, probes, KEYS);
    // Both failing; stable order preserved.
    expect(out.map((c) => c.id)).toEqual(["unknown/ctx", "dead"]);
  });
});

describe("seedFallbackList (S3/I5)", () => {
  const probes = {
    "ok/fast": { kind: "ok", latencyMs: 1000, failCount: 0, nextProbeAt: 0 },
    "ok/slow": { kind: "ok", latencyMs: 9000, failCount: 0, nextProbeAt: 0 },
    "dead/1": { kind: "probe_error", failCount: 9, nextProbeAt: 0 },
  };

  it("seeds probe-ok lanes first, excludes failing lanes", () => {
    const out = seedFallbackList(
      ["dead/1", "ok/slow", "ok/fast", "fresh/lane"],
      probes,
      4,
    );
    expect(out).toEqual(["ok/fast", "ok/slow", "fresh/lane"]);
  });

  it("respects depth", () => {
    const out = seedFallbackList(
      ["ok/fast", "ok/slow", "fresh/lane"],
      probes,
      2,
    );
    expect(out).toEqual(["ok/fast", "ok/slow"]);
  });

  it("no marks: input order, truncated (I5)", () => {
    expect(seedFallbackList(["a", "b", "c"], {}, 2)).toEqual(["a", "b"]);
  });
});
