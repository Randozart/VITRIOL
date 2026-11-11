// P1 routing registry — unit tests (written BEFORE implementation).
//
// The registry is the peer-visibility surface for multi-session routing:
// every TUI's adaptive-router writes one entry per turn decision and reads
// the others'. It must survive the jiti-isolation boundary (globalThis
// singleton) and never contain prompt text (invariant I4).
//
// Pure core (tested here): upsert / read / expire / prune / serialize.
// FS wrapper is thin (mode 600, atomic write) and covered by the
// jiti-isolation contract test, not here.

import { describe, expect, it } from "vitest";
import {
  type PeerEntry,
  type RegistryCore,
  createCore,
  upsertEntry,
  readPeers,
  pruneExpired,
  serialize,
  deserialize,
  DEFAULT_TTL_MS,
} from "./routing-registry.ts";

const NOW = 1_000_000_000_000; // fixed "now" for deterministic tests

function core(): RegistryCore {
  return createCore(NOW);
}

function entry(over: Partial<PeerEntry> = {}): PeerEntry {
  return {
    ts: NOW,
    tuiPid: 111,
    sessionId: "sess-a",
    tier: "local",
    model: "llamacpp/lapis",
    ctxTokens: 10_000,
    ...over,
  };
}

describe("createCore", () => {
  it("starts empty", () => {
    expect(readPeers(core(), NOW, 99)).toEqual([]);
  });
});

describe("upsertEntry", () => {
  it("adds a new peer", () => {
    const c = core();
    upsertEntry(c, entry());
    const peers = readPeers(c, NOW, 222);
    expect(peers).toHaveLength(1);
    expect(peers[0].sessionId).toBe("sess-a");
    expect(peers[0].tuiPid).toBe(111);
  });

  it("replaces by (tuiPid, sessionId), not by sessionId alone", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, sessionId: "s" }));
    upsertEntry(c, entry({ tuiPid: 2, sessionId: "s" }));
    expect(readPeers(c, NOW, 99)).toHaveLength(2);
    // Same pid+session twice → one entry, latest ts wins.
    upsertEntry(c, entry({ tuiPid: 1, sessionId: "s", ts: NOW + 5 }));
    expect(readPeers(c, NOW, 99).filter((p) => p.tuiPid === 1)).toHaveLength(1);
    expect(readPeers(c, NOW, 99).find((p) => p.tuiPid === 1)!.ts).toBe(NOW + 5);
  });

  it("refuses to store prompt text (I4): the entry shape has no such field", () => {
    // Compile-time guarantee is the real test; at runtime we assert the
    // serialized form contains only the declared keys.
    const c = core();
    upsertEntry(c, entry());
    const blob = serialize(c);
    for (const key of Object.keys(JSON.parse(blob))) {
      expect(["entries"]).toContain(key);
    }
    for (const e of JSON.parse(blob).entries) {
      for (const k of Object.keys(e)) {
        expect(["ts", "tuiPid", "sessionId", "tier", "model", "ctxTokens", "slotId"]).toContain(k);
      }
    }
  });
});

describe("readPeers", () => {
  it("excludes the caller (self by tuiPid)", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1 }));
    upsertEntry(c, entry({ tuiPid: 2, sessionId: "other" }));
    expect(readPeers(c, NOW, 1)).toHaveLength(1);
    expect(readPeers(c, NOW, 1)[0].tuiPid).toBe(2);
  });

  it("excludes expired entries (60s TTL default)", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, ts: NOW - (DEFAULT_TTL_MS + 1) }));
    upsertEntry(c, entry({ tuiPid: 2, sessionId: "fresh", ts: NOW }));
    expect(readPeers(c, NOW, 99)).toHaveLength(1);
    expect(readPeers(c, NOW, 99)[0].tuiPid).toBe(2);
  });

  it("treats the boundary as alive (ts == now - TTL is not yet expired)", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, ts: NOW - DEFAULT_TTL_MS }));
    expect(readPeers(c, NOW, 99)).toHaveLength(1);
  });
});

describe("pruneExpired", () => {
  it("removes dead entries, keeps live ones", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, ts: NOW - (DEFAULT_TTL_MS * 2) }));
    upsertEntry(c, entry({ tuiPid: 2, sessionId: "b", ts: NOW - 1000 }));
    const removed = pruneExpired(c, NOW);
    expect(removed).toBe(1);
    expect(readPeers(c, NOW, 99)).toHaveLength(1);
  });

  it("is idempotent", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, ts: 0 }));
    expect(pruneExpired(c, NOW)).toBe(1);
    expect(pruneExpired(c, NOW)).toBe(0);
  });
});

describe("serialize / deserialize round-trip", () => {
  it("preserves entries exactly", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, sessionId: "a", slotId: 0 }));
    upsertEntry(c, entry({ tuiPid: 2, sessionId: "b", tier: "cloud", ctxTokens: 0 }));
    const c2 = createCore(NOW);
    deserialize(c2, serialize(c));
    expect(readPeers(c2, NOW, 99).sort((a, b) => a.tuiPid - b.tuiPid)).toEqual(
      readPeers(c, NOW, 99).sort((a, b) => a.tuiPid - b.tuiPid),
    );
  });

  it("deserialize is safe on garbage (returns without throwing, empty core)", () => {
    const c = createCore(NOW);
    expect(() => deserialize(c, "{not json")).not.toThrow();
    expect(readPeers(c, NOW, 99)).toEqual([]);
  });

  it("deserialize drops entries missing required fields", () => {
    const c = createCore(NOW);
    const payload = {
      entries: [
        { ts: NOW, tuiPid: 1, sessionId: "a", tier: "local", model: "m", ctxTokens: 1 },
        { ts: NOW, tuiPid: 2 }, // missing sessionId/tier/model/ctxTokens
      ],
    };
    deserialize(c, JSON.stringify(payload));
    expect(readPeers(c, NOW, 99)).toHaveLength(1);
  });
});

describe("entry ordering for seniority (P3 input)", () => {
  it("readPeers returns peers sorted oldest-first (ts asc, tuiPid tiebreak)", () => {
    const c = core();
    upsertEntry(c, entry({ tuiPid: 1, ts: NOW + 200 }));
    upsertEntry(c, entry({ tuiPid: 2, sessionId: "b", ts: NOW }));
    upsertEntry(c, entry({ tuiPid: 3, sessionId: "c", ts: NOW + 100 }));
    const peers = readPeers(c, NOW + 300, 99);
    expect(peers.map((p) => p.tuiPid)).toEqual([2, 3, 1]);
  });
});
