// Lane spread (2026-11-14) — multi-session quota contention.
//
// Free cloud lanes enforce rate limits per lane+key. Several concurrent
// Officina sessions all running the same first-choice lane stampede it
// (429s → cooldowns → retries → user-visible "struggling"). When peers are
// visible in the routing registry, a session should prefer a lane no other
// live session is currently turning on, falling back to the ordered list
// when every lane is taken.
//
// Pure: FS access stays in the extension; this module is unit-testable.

export interface LaneRef {
  provider: string;
  id: string;
}

export interface PeerLane {
  tier: "local" | "cloud";
  model: string;
}

export function laneKey(lane: LaneRef): string {
  return `${lane.provider}/${lane.id}`;
}

/** Lanes that another LIVE session is currently using (cloud only). */
export function busyPeerLanes(peers: ReadonlyArray<PeerLane>): Set<string> {
  const busy = new Set<string>();
  for (const peer of peers) {
    if (peer.tier === "cloud" && peer.model) busy.add(peer.model);
  }
  return busy;
}

/**
 * First eligible lane not in `busy`; when every lane is busy (or the list is
 * empty of free ones) fall back to the original priority order. Never
 * reorders the caller's list.
 */
export function spreadPick<T extends LaneRef>(
  eligible: ReadonlyArray<T>,
  busy: ReadonlySet<string>,
): T | undefined {
  if (eligible.length === 0) return undefined;
  for (const lane of eligible) {
    if (!busy.has(laneKey(lane))) return lane;
  }
  return eligible[0];
}
