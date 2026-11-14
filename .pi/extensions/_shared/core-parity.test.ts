// TS <-> Rust parity (plan gate I2): the committed golden vectors in
// vitriol-core/testdata/vectors.json must produce identical decisions here.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { admitLane } from "./admission.ts";
import {
  emptyBucket,
  isExhausted,
  probeBackoffMs,
  rateAllow,
  selectProbeCandidate,
  type ProbeLane,
  type ProbeWorld,
} from "./probe-scheduler.ts";
import type { ProbeFact } from "./gateway-health.ts";

const vectors = JSON.parse(
  readFileSync(resolve(__dirname, "../../../../vitriol-core/testdata/vectors.json"), "utf8"),
) as {
  now: number;
  admission: Array<{
    promptTokens: number;
    reserveTokens: number;
    lane: { invalid: boolean; cooling: boolean; ownerPaused: boolean };
    declared: number;
    learned: number | null;
    safeBytes: number | null;
    expect: string;
  }>;
  backoff: Array<{ failCount: number; jitter: number; expectMs: number }>;
  selection: Array<{
    lanes: Array<{ id: string; owner: string }>;
    world: {
      probes: Record<string, { kind: string; okAt: number | null; until: number | null; nextProbeAt: number; failCount: number }>;
      invalid: string[];
      pausedOwners: string[];
      coolingUntil: Record<string, number>;
    };
    now: number;
    ttl: number;
    expect: { lane: string; why: string } | null;
  }>;
  exhaustion: Array<{
    lanes: Array<{ id: string; owner: string }>;
    world: {
      probes: Record<string, { kind: string; okAt: number | null; until: number | null; nextProbeAt: number; failCount: number }>;
      invalid: string[];
      pausedOwners: string[];
      coolingUntil: Record<string, number>;
    };
    now: number;
    horizon: number;
    expect: boolean;
  }>;
  rate: { perMin: number; daily: number; steps: Array<{ offsetMs: number; expectAllow: boolean }> };
};

function worldOf(spec: (typeof vectors.selection)[number]["world"]): ProbeWorld {
  const probes: Record<string, ProbeFact> = {};
  for (const [id, f] of Object.entries(spec.probes)) {
    probes[id] = {
      kind: f.kind as ProbeFact["kind"],
      okAt: f.okAt ?? undefined,
      until: f.until ?? undefined,
      failCount: f.failCount,
      nextProbeAt: f.nextProbeAt,
    };
  }
  return {
    probes,
    invalid: new Set(spec.invalid),
    pausedOwners: new Set(spec.pausedOwners),
    coolingUntil: spec.coolingUntil,
    ownerPauseUntil: {},
  };
}

describe("TS <-> Rust golden vectors", () => {
  it("admission decisions match", () => {
    for (const c of vectors.admission) {
      expect(
        admitLane({
          promptTokens: c.promptTokens,
          reserveTokens: c.reserveTokens,
          lane: c.lane,
          declared: c.declared,
          learned: c.learned,
          safeBytes: c.safeBytes,
        }),
        JSON.stringify(c),
      ).toBe(c.expect);
    }
  });

  it("probe backoff matches", () => {
    for (const c of vectors.backoff) {
      expect(probeBackoffMs(c.failCount, () => c.jitter), JSON.stringify(c)).toBe(c.expectMs);
    }
  });

  it("probe selection matches", () => {
    for (const c of vectors.selection) {
      const lanes: ProbeLane[] = c.lanes;
      const choice = selectProbeCandidate(lanes, worldOf(c.world), c.now, c.ttl);
      const actual = choice ? { lane: choice.lane.id, why: choice.why } : null;
      expect(actual, JSON.stringify(c)).toEqual(c.expect);
    }
  });

  it("exhaustion matches", () => {
    for (const c of vectors.exhaustion) {
      const lanes: ProbeLane[] = c.lanes;
      expect(isExhausted(lanes, worldOf(c.world), c.now, c.horizon), JSON.stringify(c)).toBe(
        c.expect,
      );
    }
  });

  it("rate limiter matches step by step", () => {
    let bucket = emptyBucket(vectors.now);
    for (const step of vectors.rate.steps) {
      const result = rateAllow(bucket, vectors.now + step.offsetMs, vectors.rate.perMin, vectors.rate.daily);
      expect(result.allow, JSON.stringify(step)).toBe(step.expectAllow);
      bucket = result.bucket;
    }
  });
});
