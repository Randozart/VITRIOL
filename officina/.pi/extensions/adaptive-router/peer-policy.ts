// P3 peer-aware routing policy — pure. No I/O, no pi runtime.
//
// The engine has N slots (`--parallel N`). Every TUI session wants the
// stone; when all slots are committed somebody must yield to the cloud.
// This module answers only that question, per turn boundary:
//
//   "no slot is free — should THIS session yield?"
//
// Rules, in priority order:
//   1. privacy beats utilization — a non-safe turn never yields (I2)
//   2. free capacity — nothing to arbitrate (do not yield)
//   3. subagents yield first, regardless of rank (I6)
//   4. the senior session keeps the last slot — rank 0 does not yield
//   5. everyone else yields
//
// Seniority = oldest session first (it has been here longest), then the
// largest context (its KV is the most expensive to evict and re-prefill),
// then the lowest pid (deterministic tiebreak so two sessions never both
// think they are senior).
//
// With no peers the decision is always "do not yield", so single-session
// behaviour is identical to the pre-P3 router (I1).

export type Tier = "local" | "cloud";
export type Privacy = "safe" | "sensitive" | "confidential";
export type SessionRole = "main" | "subagent";

/** This session, as the router sees it. */
export interface SelfView {
  ts: number;
  tuiPid: number;
  ctxTokens: number;
  role: SessionRole;
}

/** A peer session, as published in the routing registry. */
export interface PeerView {
  ts: number;
  tuiPid: number;
  sessionId: string;
  tier: Tier;
  model: string;
  ctxTokens: number;
  slotId?: number;
}

export interface PeerDecisionInput {
  self: SelfView;
  peers: PeerView[];
  /** Engine slots not currently processing (`slots.length - busy`). */
  freeSlots: number;
  privacy: Privacy;
}

export interface PeerDecision {
  /** True when this session should move to the cloud tier this turn. */
  demote: boolean;
  /** 0 = senior (most entitled to the stone). */
  rank: number;
  reason: string;
}

interface Participant {
  ts: number;
  tuiPid: number;
  ctxTokens: number;
  isSelf: boolean;
}

/**
 * Rank order of the local-tier participants: oldest first, then largest
 * context, then lowest pid. Peers already on the cloud tier are excluded —
 * they hold no slot, so they cannot contest one.
 */
function localParticipants(self: SelfView, peers: PeerView[]): Participant[] {
  const list: Participant[] = [
    { ts: self.ts, tuiPid: self.tuiPid, ctxTokens: self.ctxTokens, isSelf: true },
  ];
  for (const peer of peers) {
    if (peer.tier !== "local") continue;
    list.push({ ts: peer.ts, tuiPid: peer.tuiPid, ctxTokens: peer.ctxTokens, isSelf: false });
  }
  list.sort((a, b) => a.ts - b.ts || b.ctxTokens - a.ctxTokens || a.tuiPid - b.tuiPid);
  return list;
}

/** Index of this session in the seniority order (0 = most senior). */
export function seniorityRank(self: SelfView, peers: PeerView[]): number {
  const list = localParticipants(self, peers);
  const idx = list.findIndex((p) => p.isSelf);
  return idx < 0 ? 0 : idx;
}

/**
 * The turn-boundary peer decision. Pure and total: every input yields a
 * decision, and `demote: false` is always the safe answer.
 */
export function decidePeerYield(input: PeerDecisionInput): PeerDecision {
  const { self, peers, freeSlots, privacy } = input;
  const rank = seniorityRank(self, peers);
  const participants = localParticipants(self, peers).length;

  // 1. Privacy beats utilization.
  if (privacy !== "safe") {
    return { demote: false, rank, reason: `${privacy} — local only (privacy beats utilization)` };
  }

  // 2. Capacity available: nothing to arbitrate.
  if (freeSlots > 0) {
    return { demote: false, rank, reason: `${freeSlots} free slot(s) — stay local` };
  }

  // 3. Side tasks are the cheapest thing to move.
  if (self.role === "subagent") {
    return { demote: true, rank, reason: "subagent under contention — yields first" };
  }

  // 4. The senior session keeps the last slot.
  if (rank === 0) {
    return { demote: false, rank, reason: "senior session — keeps the slot" };
  }

  // 5. Everyone else makes room.
  return {
    demote: true,
    rank,
    reason: `no free slot — rank ${rank} of ${participants} yields to the senior session`,
  };
}
