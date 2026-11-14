import { describe, expect, it } from "vitest";
import type { ProbeFact } from "./gateway-health.ts";
import {
  emptyBucket,
  isExhausted,
  isVerified,
  probeBackoffMs,
  rateAllow,
  selectProbeCandidate,
  type ProbeLane,
  type ProbeWorld,
} from "./probe-scheduler.ts";

const NOW = 1_790_000_000_000;
const lanes: ProbeLane[] = [
  { id: "dahl/MiniMaxAI/MiniMax-M2.7", owner: "dahl" },
  { id: "freeinference/deepseek-v4-flash", owner: "freeinference" },
  { id: "agentrouter/gpt-5.6-sol", owner: "agentrouter" },
];

const world = (over: Partial<ProbeWorld> = {}): ProbeWorld => ({
  probes: {},
  invalid: new Set(),
  pausedOwners: new Set(),
  coolingUntil: {},
  ownerPauseUntil: {},
  ...over,
});

const fact = (over: Partial<ProbeFact>): ProbeFact => ({
  kind: "ok",
  failCount: 0,
  nextProbeAt: 0,
  ...over,
});

describe("probeBackoffMs", () => {
  it("follows the ladder and clamps at 10 minutes", () => {
    expect(probeBackoffMs(1, () => 0.5)).toBe(30_000);
    expect(probeBackoffMs(2, () => 0.5)).toBe(60_000);
    expect(probeBackoffMs(3, () => 0.5)).toBe(120_000);
    expect(probeBackoffMs(4, () => 0.5)).toBe(300_000);
    expect(probeBackoffMs(9, () => 0.5)).toBe(600_000);
  });
  it("applies ±20% jitter", () => {
    expect(probeBackoffMs(1, () => 0)).toBe(24_000);
    expect(probeBackoffMs(1, () => 1)).toBe(36_000);
  });
});

describe("selectProbeCandidate", () => {
  it("prefers unverified lanes, then newly expired, then stale", () => {
    const w = world({
      probes: {
        "dahl/MiniMaxAI/MiniMax-M2.7": fact({ kind: "cooling", until: NOW - 5_000, nextProbeAt: NOW }),
        "freeinference/deepseek-v4-flash": fact({ okAt: NOW - 11 * 60_000, nextProbeAt: NOW }),
      },
    });
    expect(selectProbeCandidate(lanes, w, NOW)?.lane.id).toBe("agentrouter/gpt-5.6-sol");
    const w2 = world({
      probes: {
        "dahl/MiniMaxAI/MiniMax-M2.7": fact({ kind: "cooling", until: NOW - 5_000, nextProbeAt: NOW }),
        "freeinference/deepseek-v4-flash": fact({ okAt: NOW - 1_000, nextProbeAt: NOW }),
        "agentrouter/gpt-5.6-sol": fact({ okAt: NOW - 1_000, nextProbeAt: NOW }),
      },
    });
    expect(selectProbeCandidate(lanes, w2, NOW)?.why).toBe("cooldown-expired");
  });

  it("skips invalid, paused and actively cooling lanes", () => {
    const w = world({
      invalid: new Set(["agentrouter/gpt-5.6-sol"]),
      pausedOwners: new Set(["freeinference"]),
      coolingUntil: { "dahl/MiniMaxAI/MiniMax-M2.7": NOW + 60_000 },
    });
    expect(selectProbeCandidate(lanes, w, NOW)).toBeUndefined();
  });

  it("does not re-probe a freshly verified lane", () => {
    const w = world({
      probes: Object.fromEntries(lanes.map((l) => [l.id, fact({ okAt: NOW - 1_000 })])),
    });
    expect(selectProbeCandidate(lanes, w, NOW)).toBeUndefined();
    expect(isVerified(lanes[0]!.id, w.probes, NOW)).toBe(true);
  });

  it("retries a failed lane when its backoff elapsed", () => {
    const w = world({
      probes: {
        ...Object.fromEntries(
          lanes.map((l) => [l.id, fact({ okAt: NOW - 1_000 })]),
        ),
        "agentrouter/gpt-5.6-sol": fact({ kind: "no_fit", failCount: 1, nextProbeAt: NOW - 1 }),
      },
    });
    expect(selectProbeCandidate(lanes, w, NOW)).toMatchObject({
      why: "retry",
      lane: { id: "agentrouter/gpt-5.6-sol" },
    });
  });
});

describe("isExhausted", () => {
  it("is false while any lane can recover", () => {
    const w = world({ coolingUntil: { "dahl/MiniMaxAI/MiniMax-M2.7": NOW + 60_000 } });
    expect(isExhausted(lanes, w, NOW)).toBe(false);
  });
  it("is true when every lane is invalid/paused or beyond the horizon", () => {
    expect(
      isExhausted(
        lanes,
        world({
          invalid: new Set([lanes[0]!.id]),
          pausedOwners: new Set(["freeinference"]),
          coolingUntil: { "agentrouter/gpt-5.6-sol": NOW + 31 * 60_000 },
        }),
        NOW,
      ),
    ).toBe(true);
    expect(isExhausted([], world(), NOW)).toBe(true);
  });
});

describe("rateAllow", () => {
  it("enforces per-minute and daily caps", () => {
    let bucket = emptyBucket(NOW);
    for (let i = 0; i < 3; i += 1) {
      const r = rateAllow(bucket, NOW + i, 3, 100);
      expect(r.allow).toBe(true);
      bucket = r.bucket;
    }
    expect(rateAllow(bucket, NOW + 10, 3, 100).allow).toBe(false);
    const tomorrow = rateAllow({ ...bucket, recent: [] }, NOW + 86_400_000, 3, 100);
    expect(tomorrow.allow).toBe(true);
    expect(tomorrow.bucket.dayCount).toBe(1);
  });
});
