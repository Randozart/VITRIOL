// probe-order — spill candidate ordering driven by background probe marks
// (2026-11-21, plan free-inference-probe-routing-2026-11-21.md S1).
//
// The cloud-probe already knows which free lanes are alive; until now that
// knowledge was write-only. This pure function turns probe facts into a
// candidate order: verified lanes first (fastest first), unprobed lanes in
// catalog order next, context-limited lanes that FIT the request after
// that, and failing/cooling lanes dead last.
//
// PURE: no fs, no timers. The caller supplies the probe map.
//
// I1: an absent/empty probe map must return the input order unchanged —
// parity by construction.

/** Minimal shape of a probe fact (structural subset of gateway-health). */
export interface ProbeFactLite {
  kind: string;
  latencyMs?: number;
  failAt?: number;
  failCount?: number;
  until?: number;
}

/** How to read lane identity + learned limit off an arbitrary candidate. */
export interface LaneKeyed<T> {
  /** Stable lane id — the key probe facts are stored under. */
  laneId: (item: T) => string;
  /** Learned context limit for the item, if known. */
  limitTokens?: (item: T) => number | undefined;
}

/** Rank buckets: lower sorts first. */
const RANK = {
  ok: 0,
  unprobed: 1,
  context_fit: 2,
  failing: 3,
} as const;

/**
 * Order spill candidates by probe knowledge (S1). Generic over the
 * candidate shape via a LaneKeyed extractor.
 *  - probe-ok first, ascending latencyMs (ties: input order);
 *  - unprobed next, input order (I1);
 *  - context_limit lanes with a learned limit after that (I3);
 *  - everything failing/cooling last.
 * Ordering is stable: equal keys never reorder relative to the input.
 */
export function orderCandidates<T>(
  candidates: ReadonlyArray<T>,
  probes: Record<string, ProbeFactLite> = {},
  keys: LaneKeyed<T> = { laneId: (item) => String((item as { id?: string }).id) },
  now: number = Date.now(),
): T[] {
  const limitOf = keys.limitTokens ?? (() => undefined);
  const decorated = candidates.map((candidate, index) => {
    const id = keys.laneId(candidate);
    const fact = probes[id];
    let bucket: number;
    if (!fact) bucket = RANK.unprobed;
    else if (fact.kind === "ok") bucket = RANK.ok;
    else if (fact.kind === "context_limit") {
      bucket = typeof limitOf(candidate) === "number"
        ? RANK.context_fit
        : RANK.failing;
    } else bucket = RANK.failing;
    return {
      candidate,
      index,
      bucket,
      latency: fact?.latencyMs ?? Number.POSITIVE_INFINITY,
    };
  });
  decorated.sort((a, b) => {
    if (a.bucket !== b.bucket) return a.bucket - b.bucket;
    if (a.bucket === RANK.ok && a.latency !== b.latency) {
      return a.latency - b.latency;
    }
    return a.index - b.index;
  });
  return decorated.map((d) => d.candidate);
}
