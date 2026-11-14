// Cloud-probe scheduler (2026-11-14) — PURE logic, no I/O.
//
// One leader session probes cloud lanes in the background and publishes facts
// (gateway-health.probes) that every router reads before a request. This module
// owns the policy: what to probe next, how long to wait after failures, when to
// stop (truly exhausted) and the per-minute/day rate limits. The extension only
// does fetch + timers; vitriol-core mirrors these functions for parity (I2).

import type { ProbeFact } from "./gateway-health.ts";

export interface ProbeLane {
  /** Full lane id inside the provider, e.g. "dahl/MiniMaxAI/MiniMax-M2.7". */
  id: string;
  /** Owner id ("dahl"). */
  owner: string;
}

export interface ProbeWorld {
  probes: Record<string, ProbeFact>;
  invalid: ReadonlySet<string>;
  pausedOwners: ReadonlySet<string>;
  /** Lane cooldown expiry from provider-health (ms epoch). */
  coolingUntil: Record<string, number>;
  /** Owner pause expiry (ms epoch), for horizon checks. */
  ownerPauseUntil: Record<string, number>;
}

export const BACKOFF_MS: readonly number[] = [30_000, 60_000, 120_000, 300_000, 600_000];
export const DEFAULT_VERIFIED_TTL_MS = 10 * 60_000;
export const DEFAULT_EXHAUSTED_HORIZON_MS = 30 * 60_000;

/** Exponential backoff for a lane's next probe (jitter in [0.8, 1.2]). */
export function probeBackoffMs(failCount: number, rng: () => number = Math.random): number {
  const idx = Math.max(0, Math.min(BACKOFF_MS.length - 1, Math.floor(failCount) - 1));
  const base = BACKOFF_MS[Number.isFinite(idx) ? idx : 0]!;
  const jitter = 0.8 + Math.min(1, Math.max(0, rng())) * 0.4;
  return Math.round(base * jitter);
}

export function isVerified(
  laneId: string,
  probes: Record<string, ProbeFact>,
  now: number,
  ttlMs = DEFAULT_VERIFIED_TTL_MS,
): boolean {
  const fact = probes[laneId];
  return !!fact && fact.kind === "ok" && Number(fact.okAt ?? 0) > 0 && now - Number(fact.okAt) < ttlMs;
}

export type ProbeWhy = "unverified" | "cooldown-expired" | "stale" | "retry";

export interface ProbeChoice {
  lane: ProbeLane;
  why: ProbeWhy;
}

function blocked(lane: ProbeLane, world: ProbeWorld, now: number): boolean {
  if (world.invalid.has(lane.id)) return true;
  if (world.pausedOwners.has(lane.owner)) return true;
  const cooling = world.coolingUntil[lane.id] ?? 0;
  if (cooling > now + 1_000) return true; // actively cooling — do not burn quota
  const fact = world.probes[lane.id];
  if (fact && Number(fact.nextProbeAt) > now) return true;
  return false;
}

/**
 * Highest-value lane to probe now: unverified → cooldown just expired →
 * verified-but-stale → retry due. Ties rotate by a stable lane hash.
 */
export function selectProbeCandidate(
  lanes: ReadonlyArray<ProbeLane>,
  world: ProbeWorld,
  now: number,
  ttlMs = DEFAULT_VERIFIED_TTL_MS,
): ProbeChoice | undefined {
  const rank = (lane: ProbeLane): { priority: number; why: ProbeWhy } | undefined => {
    if (blocked(lane, world, now)) return undefined;
    const fact = world.probes[lane.id];
    if (!fact) return { priority: 0, why: "unverified" };
    if (fact.kind === "ok") {
      const okAt = Number(fact.okAt ?? 0);
      if (now - okAt >= ttlMs) return { priority: 2, why: "stale" };
      return undefined; // fresh verification — leave it alone
    }
    const until = Number(fact.until ?? 0);
    if (until > 0 && until <= now + 30_000) return { priority: 1, why: "cooldown-expired" };
    if (Number(fact.nextProbeAt) <= now) return { priority: 3, why: "retry" };
    return undefined;
  };
  const choices = lanes
    .map((lane) => ({ lane, ...(rank(lane) as { priority: number; why: ProbeWhy }) }))
    .filter((c) => c.priority !== undefined)
    .sort((a, b) => {
      if (a.priority !== b.priority) return a.priority - b.priority;
      return stableHash(a.lane.id) - stableHash(b.lane.id);
    });
  const best = choices[0];
  return best ? { lane: best.lane, why: best.why } : undefined;
}

function stableHash(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/**
 * Truly exhausted: every lane is invalid/owner-paused, or every remaining
 * cooldown/pause ends beyond the horizon. Then the extension drops to the slow
 * heartbeat (still probing, just rarely) — it never stops while recovery is
 * plausible (operator requirement).
 */
export function isExhausted(
  lanes: ReadonlyArray<ProbeLane>,
  world: ProbeWorld,
  now: number,
  horizonMs = DEFAULT_EXHAUSTED_HORIZON_MS,
): boolean {
  if (lanes.length === 0) return true;
  let any = false;
  for (const lane of lanes) {
    if (world.invalid.has(lane.id)) continue;
    if (world.pausedOwners.has(lane.owner)) continue;
    any = true;
    const cooling = world.coolingUntil[lane.id] ?? 0;
    if (cooling <= now + horizonMs) return false;
  }
  if (!any) return true;
  // All remaining lanes have cooldowns beyond the horizon.
  return true;
}

export interface RateBucket {
  /** Probe timestamps within the last minute. */
  recent: number[];
  /** UTC day key (YYYY-MM-DD) and count. */
  dayKey: string;
  dayCount: number;
}

export function emptyBucket(now: number): RateBucket {
  return { recent: [], dayKey: new Date(now).toISOString().slice(0, 10), dayCount: 0 };
}

export function rateAllow(
  bucket: RateBucket,
  now: number,
  perMin: number,
  daily: number,
): { allow: boolean; bucket: RateBucket } {
  const dayKey = new Date(now).toISOString().slice(0, 10);
  const dayCount = dayKey === bucket.dayKey ? bucket.dayCount : 0;
  const recent = bucket.recent.filter((t) => now - t < 60_000);
  const allow = recent.length < perMin && dayCount < daily;
  return {
    allow,
    bucket: {
      recent: allow ? [...recent, now] : recent,
      dayKey,
      dayCount: allow ? dayCount + 1 : dayCount,
    },
  };
}
