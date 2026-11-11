// P3 peer-aware routing policy — unit tests (written BEFORE implementation).
//
// Answers one question at a turn boundary: "the engine's slots are all
// committed — should THIS session yield to the cloud so its peers keep the
// stone?"
//
// Rules (see .opencode/plans/multi-session-routing-2026-11-10.md):
//   - privacy beats utilization: non-safe turns never yield (I2)
//   - failover/engine-down is decided earlier, not here (I3)
//   - rank 0 (the senior session) keeps the last slot: its KV is the most
//     expensive to evict and re-prefill
//   - subagents yield first, regardless of rank (I6)
//   - with no peers the decision is "do not yield", so single-session
//     behaviour is byte-identical to today (I1)

import { describe, expect, it } from "vitest";
import {
  decidePeerYield,
  seniorityRank,
  type PeerView,
  type SelfView,
} from "./peer-policy.ts";

const T0 = 1_000_000_000_000;

function self(over: Partial<SelfView> = {}): SelfView {
  return { ts: T0, tuiPid: 100, ctxTokens: 10_000, role: "main", ...over };
}

function peer(over: Partial<PeerView> = {}): PeerView {
  return {
    ts: T0,
    tuiPid: 200,
    sessionId: "peer",
    tier: "local",
    model: "llamacpp/Lapis Occultus",
    ctxTokens: 10_000,
    ...over,
  };
}

/** Promote a peer to the calling session's view (same identity fields). */
function asSelf(p: PeerView, role: SelfView["role"] = "main"): SelfView {
  return { ts: p.ts, tuiPid: p.tuiPid, ctxTokens: p.ctxTokens, role };
}

describe("seniorityRank", () => {
  it("is 0 with no peers (single session is always senior)", () => {
    expect(seniorityRank(self(), [])).toBe(0);
  });

  it("ranks the oldest session first", () => {
    const me = self({ ts: T0 + 500, tuiPid: 100 });
    const older = peer({ ts: T0, tuiPid: 200 });
    expect(seniorityRank(me, [older])).toBe(1);
    expect(seniorityRank(asSelf(older), [peer({ ts: T0 + 500, tuiPid: 100 })])).toBe(0);
  });

  it("breaks an age tie by larger context (more expensive to evict)", () => {
    const me = self({ ts: T0, ctxTokens: 5_000, tuiPid: 100 });
    const fat = peer({ ts: T0, ctxTokens: 120_000, tuiPid: 200 });
    expect(seniorityRank(me, [fat])).toBe(1);
    expect(seniorityRank(asSelf(fat), [peer({ ts: T0, ctxTokens: 5_000, tuiPid: 100 })])).toBe(0);
  });

  it("breaks a full tie deterministically by tuiPid", () => {
    const low = peer({ ts: T0, ctxTokens: 10_000, tuiPid: 100 });
    const high = peer({ ts: T0, ctxTokens: 10_000, tuiPid: 900 });
    expect(seniorityRank(asSelf(high), [low])).toBe(1);
    expect(seniorityRank(asSelf(low), [high])).toBe(0);
  });

  it("ignores peers already on the cloud tier (they hold no slot)", () => {
    const me = self({ ts: T0 + 100 });
    const cloudPeer = peer({ ts: T0, tier: "cloud" });
    expect(seniorityRank(me, [cloudPeer])).toBe(0);
  });

  it("handles three participants", () => {
    const mePeer = peer({ ts: T0 + 200, tuiPid: 300, sessionId: "me" });
    const a = peer({ ts: T0, tuiPid: 100, sessionId: "a" });
    const b = peer({ ts: T0 + 100, tuiPid: 200, sessionId: "b" });
    expect(seniorityRank(asSelf(mePeer), [a, b])).toBe(2);
    expect(seniorityRank(asSelf(a), [b, mePeer])).toBe(0);
    expect(seniorityRank(asSelf(b), [a, mePeer])).toBe(1);
  });
});

describe("decidePeerYield", () => {
  it("does not yield when a slot is free", () => {
    const d = decidePeerYield({
      self: self({ ts: T0 + 999 }),
      peers: [peer({ ts: T0 })],
      freeSlots: 1,
      privacy: "safe",
    });
    expect(d.demote).toBe(false);
    expect(d.reason).toMatch(/slot/i);
  });

  it("does not yield with no peers, even when no slot is free (I1 parity)", () => {
    const d = decidePeerYield({ self: self(), peers: [], freeSlots: 0, privacy: "safe" });
    expect(d.demote).toBe(false);
  });

  it("yields when no slot is free and a senior peer holds the stone", () => {
    const d = decidePeerYield({
      self: self({ ts: T0 + 500 }),
      peers: [peer({ ts: T0 })],
      freeSlots: 0,
      privacy: "safe",
    });
    expect(d.demote).toBe(true);
    expect(d.rank).toBe(1);
    expect(d.reason).toMatch(/slot|rank/i);
  });

  it("keeps the slot as the senior session", () => {
    const d = decidePeerYield({
      self: self({ ts: T0, ctxTokens: 120_000 }),
      peers: [peer({ ts: T0 + 500, ctxTokens: 1_000 })],
      freeSlots: 0,
      privacy: "safe",
    });
    expect(d.demote).toBe(false);
    expect(d.rank).toBe(0);
  });

  it("never yields a confidential turn (I2)", () => {
    const d = decidePeerYield({
      self: self({ ts: T0 + 500 }),
      peers: [peer({ ts: T0 })],
      freeSlots: 0,
      privacy: "confidential",
    });
    expect(d.demote).toBe(false);
    expect(d.reason).toMatch(/confidential|privacy|local/i);
  });

  it("never yields a sensitive turn (I2)", () => {
    const d = decidePeerYield({
      self: self({ ts: T0 + 500 }),
      peers: [peer({ ts: T0 })],
      freeSlots: 0,
      privacy: "sensitive",
    });
    expect(d.demote).toBe(false);
  });

  it("yields a subagent first even when it is the senior session (I6)", () => {
    const d = decidePeerYield({
      self: self({ ts: T0, ctxTokens: 120_000, role: "subagent" }),
      peers: [peer({ ts: T0 + 500 })],
      freeSlots: 0,
      privacy: "safe",
    });
    expect(d.demote).toBe(true);
    expect(d.reason).toMatch(/subagent/i);
  });

  it("does not yield a subagent when a slot is free", () => {
    const d = decidePeerYield({
      self: self({ role: "subagent" }),
      peers: [peer({ ts: T0 })],
      freeSlots: 2,
      privacy: "safe",
    });
    expect(d.demote).toBe(false);
  });

  it("ignores cloud-tier peers when deciding (they hold no slot)", () => {
    const d = decidePeerYield({
      self: self({ ts: T0 + 500 }),
      peers: [peer({ ts: T0, tier: "cloud" })],
      freeSlots: 0,
      privacy: "safe",
    });
    expect(d.demote).toBe(false);
    expect(d.rank).toBe(0);
  });

  it("reports the rank so the caller can show why it was demoted", () => {
    const d = decidePeerYield({
      self: self({ ts: T0 + 900 }),
      peers: [
        peer({ ts: T0, tuiPid: 201, sessionId: "a" }),
        peer({ ts: T0 + 400, tuiPid: 202, sessionId: "b" }),
      ],
      freeSlots: 0,
      privacy: "safe",
    });
    expect(d.demote).toBe(true);
    expect(d.rank).toBe(2);
  });
});
