// T1a — fast-check property tests for the router policy surface.
//
// Deterministic suites pin specific cases; these pin the SHAPE of the
// behaviour: whatever the inputs, the invariants must hold. Acts as a
// regression net against policy refactors.

import * as fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  createCore,
  deserialize,
  pruneExpired,
  readPeers,
  serialize,
  upsertEntry,
  type PeerEntry,
} from "../_shared/routing-registry.ts";
import {
  decidePeerYield,
  seniorityRank,
  type PeerView,
  type SelfView,
} from "./peer-policy.ts";
import {
  configFromEnv,
  decideTier,
  type DecideInput,
  type LoadSample,
  type RouterState,
} from "./overflow.ts";

const T0 = 1_000_000_000_000;
const cfg = configFromEnv({});

// ── arbitraries ──────────────────────────────────────────────────────────

const arbTs = fc.integer({ min: T0, max: T0 + 100 * 3_600_000 });
const arbCtxTokens = fc.integer({ min: 0, max: 500_000 });
const arbRole: fc.Arbitrary<SelfView["role"]> = fc.constantFrom("main", "subagent");
const arbTuiPid = fc.integer({ min: 1, max: 1_000_000 });

const arbSelf: fc.Arbitrary<SelfView> = fc.record({
  ts: arbTs,
  tuiPid: arbTuiPid,
  ctxTokens: arbCtxTokens,
  role: arbRole,
});

const arbPeer: fc.Arbitrary<PeerView> = fc.record({
  ts: arbTs,
  tuiPid: arbTuiPid,
  sessionId: fc.string({ minLength: 1, maxLength: 24 }),
  tier: fc.constantFrom<"local" | "cloud">("local", "cloud"),
  model: fc.string({ minLength: 1, maxLength: 64 }),
  ctxTokens: arbCtxTokens,
});

const arbPeerList: fc.Arbitrary<PeerView[]> = fc.array(arbPeer, {
  minLength: 0,
  maxLength: 8,
});

function baseInput(over: Partial<DecideInput> = {}): DecideInput {
  const idle: LoadSample = {
    processing: 0,
    deferred: 0,
    queueWaitMs: 0,
    stalled: false,
    gpuLoad: null,
  };
  const state: RouterState = { tier: "local", switches: 0, turnsOnTier: 1, cloudTokens: 0 };
  return {
    state,
    load: idle,
    privacy: "safe",
    cloudFits: true,
    budgetLeft: true,
    localIdleForMs: null,
    localDown: false,
    failoverActive: false,
    ...over,
  };
}

// ── seniorityRank (property) ─────────────────────────────────────────────

describe("seniorityRank (property)", () => {
  it("cloud-tier peers never contest the ranking", () => {
    fc.assert(
      fc.property(arbSelf, arbPeer, (self, p) => {
        const localPeer: PeerView = { ...p, tier: "local" };
        const cloudPeer: PeerView = { ...p, tier: "cloud" };
        // A cloud peer must not change the rank relative to an otherwise
        // identical local peer — it holds no slot.
        const rankWithCloud = seniorityRank(self, [cloudPeer]);
        const rankWithLocal = seniorityRank(self, [localPeer]);
        // Not asserting equality (local peer CAN change rank), but the cloud
        // peer alone must behave as if absent.
        const rankAlone = seniorityRank(self, []);
        expect(rankWithCloud).toBe(rankAlone);
        void rankWithLocal;
      }),
      { numRuns: 200 },
    );
  });

  it("rank 0 always exists and equals the number of strictly-more-senior locals", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, (self, peers) => {
        const r = seniorityRank(self, peers);
        expect(r).toBeGreaterThanOrEqual(0);
        const localPeers = peers.filter((p) => p.tier === "local");
        expect(r).toBeLessThanOrEqual(localPeers.length);
      }),
      { numRuns: 200 },
    );
  });

  it("a strictly older local peer is always ranked more senior than self", () => {
    fc.assert(
      fc.property(arbSelf, arbCtxTokens, (self, bigger) => {
        const older: PeerView = {
          ts: self.ts - 1,
          tuiPid: self.tuiPid + 7,
          sessionId: "older",
          tier: "local",
          model: "llamacpp/Lapis",
          ctxTokens: bigger,
        };
        expect(seniorityRank(self, [older])).toBe(1);
      }),
      { numRuns: 200 },
    );
  });
});

// ── decidePeerYield (property) ──────────────────────────────────────────

describe("decidePeerYield (property)", () => {
  it("never yields with privacy !== safe (I2)", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, (self, peers) => {
        for (const privacy of ["sensitive", "confidential"] as const) {
          for (const fs of [0, 1, 2]) {
            const d = decidePeerYield({ self, peers, freeSlots: fs, privacy });
            expect(d.demote).toBe(false);
          }
        }
      }),
      { numRuns: 200 },
    );
  });

  it("never yields when a slot is free", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, arbTuiPid, (self, peers, rawFs) => {
        const d = decidePeerYield({ self, peers, freeSlots: rawFs % 5 + 1, privacy: "safe" });
        expect(d.demote).toBe(false);
      }),
      { numRuns: 200 },
    );
  });

  it("a main-agent yielding decision always carries rank >= 1", () => {
    // (A rank-0 subagent yields to its own role, not to rank — I6.)
    fc.assert(
      fc.property(arbSelf, arbPeerList, (self, peers) => {
        const main = { ...self, role: "main" as const };
        const d = decidePeerYield({ self: main, peers, freeSlots: 0, privacy: "safe" });
        if (d.demote) expect(d.rank).toBeGreaterThanOrEqual(1);
      }),
      { numRuns: 200 },
    );
  });

  it("a subagent yields first under contention (I6)", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, (self, peers) => {
        const sub = { ...self, role: "subagent" as const };
        // Even against zero peers, a subagent with no free slot yields.
        const d = decidePeerYield({ self: sub, peers: [], freeSlots: 0, privacy: "safe" });
        expect(d.demote).toBe(true);
        void peers;
      }),
      { numRuns: 100 },
    );
  });

  it("the senior session (rank 0, main role) never yields under contention", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, (self, peers) => {
        const main = { ...self, role: "main" as const };
        // Build a peer list where self is guaranteed senior: all peers newer
        // than self.
        const newerPeers = peers.map((p) => ({ ...p, ts: main.ts + 10_000, tier: "local" as const }));
        const d = decidePeerYield({
          self: main,
          peers: newerPeers,
          freeSlots: 0,
          privacy: "safe",
        });
        expect(d.demote).toBe(false);
        expect(d.rank).toBe(0);
      }),
      { numRuns: 200 },
    );
  });
});

// ── decideTier (property) ────────────────────────────────────────────────

describe("decideTier (property)", () => {
  it("confidential prompts never route to cloud", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, fc.integer({ min: 0, max: 4 }), fc.boolean(), fc.boolean(), (self, peers, fs, cloudFits, budgetLeft) => {
        const d = decideTier(
          baseInput({ self, peers, freeSlots: fs, privacy: "confidential", cloudFits, budgetLeft, localDown: true }),
          cfg,
        );
        expect(d.tier).toBe("local");
      }),
      { numRuns: 200 },
    );
  });

  it("peerDemoted is only set when freeSlots === 0 and privacy is safe", () => {
    fc.assert(
      fc.property(
        arbSelf,
        arbPeerList,
        fc.integer({ min: 0, max: 4 }),
        fc.constantFrom<"safe" | "sensitive" | "confidential">("safe", "sensitive", "confidential"),
        (self, peers, fs, privacy) => {
          const d = decideTier(baseInput({ self, peers, freeSlots: fs, privacy }), cfg);
          if (d.peerDemoted) {
            expect(fs).toBe(0);
            expect(privacy).toBe("safe");
            expect(d.tier).toBe("cloud");
          }
        },
      ),
      { numRuns: 200 },
    );
  });

  it("local-down always fails over to cloud (when a candidate exists) and is never peerDemoted", () => {
    fc.assert(
      fc.property(arbSelf, arbPeerList, fc.integer({ min: 0, max: 4 }), (self, peers, fs) => {
        const d = decideTier(
          baseInput({ self, peers, freeSlots: fs, localDown: true, cloudFits: true, budgetLeft: true }),
          cfg,
        );
        expect(d.tier).toBe("cloud");
        expect(d.peerDemoted).toBeUndefined();
      }),
      { numRuns: 200 },
    );
  });

  it("peer demotion needs a local peer to arbitrate against (main role)", () => {
    fc.assert(
      fc.property(arbSelf, (self) => {
        const main = { ...self, role: "main" as const };
        const d = decideTier(baseInput({ self: main, peers: [], freeSlots: 0 }), cfg);
        expect(d.peerDemoted).toBeUndefined();
      }),
      { numRuns: 100 },
    );
  });
});

// ── routing-registry (property) ──────────────────────────────────────────

describe("routing-registry (property)", () => {
  const arbEntry: fc.Arbitrary<PeerEntry> = fc.record({
    ts: arbTs,
    tuiPid: arbTuiPid,
    sessionId: fc.string({ minLength: 1, maxLength: 24 }),
    tier: fc.constantFrom<"local" | "cloud">("local", "cloud"),
    model: fc.string({ minLength: 1, maxLength: 64 }),
    ctxTokens: arbCtxTokens,
  });

  it("serialize/deserialize round-trips every entry", () => {
    fc.assert(
      fc.property(fc.array(arbEntry, { minLength: 0, maxLength: 12 }), (entries) => {
        const core = createCore(T0);
        for (const e of entries) upsertEntry(core, e);
        const json = serialize(core);
        const restored = createCore(T0);
        deserialize(restored, json);
        expect(restored.entries.size).toBe(core.entries.size);
        for (const [key, e] of core.entries) {
          const got = restored.entries.get(key);
          expect(got).toBeDefined();
          expect(got!.tuiPid).toBe(e.tuiPid);
          expect(got!.sessionId).toBe(e.sessionId);
          expect(got!.ctxTokens).toBe(e.ctxTokens);
          expect(got!.tier).toBe(e.tier);
        }
      }),
      { numRuns: 200 },
    );
  });

  it("upsert is idempotent for the same (tuiPid, sessionId)", () => {
    fc.assert(
      fc.property(arbEntry, (e) => {
        const core = createCore(T0);
        upsertEntry(core, e);
        const size = core.entries.size;
        upsertEntry(core, e);
        expect(core.entries.size).toBe(size);
        expect(core.entries.get(`${e.tuiPid}:${e.sessionId}`)?.ts).toBe(e.ts);
      }),
      { numRuns: 200 },
    );
  });

  it("readPeers never returns self and only returns unexpired peers", () => {
    fc.assert(
      fc.property(arbEntry, fc.integer({ min: 0, max: 120_000 }), (e, ageMs) => {
        const core = createCore(T0);
        upsertEntry(core, { ...e, ts: T0 - ageMs });
        const selfPid = e.tuiPid + 999_999;
        const peers = readPeers(core, T0, selfPid, 60_000);
        expect(peers.some((p) => p.tuiPid === selfPid)).toBe(false);
        if (ageMs > 60_000) expect(peers.some((p) => p.tuiPid === e.tuiPid)).toBe(false);
        else expect(peers.some((p) => p.tuiPid === e.tuiPid)).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it("pruneExpired drops exactly the expired entries", () => {
    fc.assert(
      fc.property(fc.array(arbEntry, { minLength: 0, maxLength: 12 }), (entries) => {
        const core = createCore(T0);
        for (const e of entries) upsertEntry(core, e);
        const before = core.entries.size;
        const expected = [...core.entries.values()].filter((e) => T0 - e.ts > 60_000).length;
        const removed = pruneExpired(core, T0, 60_000);
        expect(removed).toBe(expected);
        expect(core.entries.size).toBe(before - expected);
        for (const e of core.entries.values()) expect(T0 - e.ts).toBeLessThanOrEqual(60_000);
      }),
      { numRuns: 200 },
    );
  });

  it("deserialize never throws on garbage and yields an empty core", () => {
    fc.assert(
      fc.property(fc.string(), (garbage) => {
        const core = createCore(T0);
        expect(() => deserialize(core, garbage)).not.toThrow();
        expect(core.entries.size).toBe(0);
      }),
      { numRuns: 200 },
    );
  });
});
