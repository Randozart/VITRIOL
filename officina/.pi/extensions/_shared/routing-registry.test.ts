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
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  gcExpiredShards,
  type PeerEntry,
  type RegistryCore,
  createCore,
  upsertEntry,
  readPeers,
  pruneExpired,
  serialize,
  deserialize,
  DEFAULT_TTL_MS,
  readLivePeers,
  removeRegistryShard,
  writeRegistryEntry,
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
        expect(["ts", "tuiPid", "sessionId", "tier", "model", "ctxTokens", "slotId", "role"]).toContain(k);
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

// ── Shard-mode FS layer (2026-11-11, I7) ──────────────────────────────────
//
// Production writes one shard per pid into OFFICINA_ROUTING_REGISTRY_DIR;
// readers glob-merge. The I7 pin: concurrent publishers — each doing what
// the OLD whole-file read-modify-write did — can no longer drop each
// other's entries, because a shard writer never touches peers' files.
describe("shard mode", () => {
  const NOW2 = Date.now();

  function withShardDir<T>(fn: (dir: string) => T): T {
    const dir = mkdtempSync(join(tmpdir(), "registry-shards-"));
    const prev = process.env.OFFICINA_ROUTING_REGISTRY_DIR;
    process.env.OFFICINA_ROUTING_REGISTRY_DIR = dir;
    try {
      return fn(dir);
    } finally {
      if (prev === undefined) delete process.env.OFFICINA_ROUTING_REGISTRY_DIR;
      else process.env.OFFICINA_ROUTING_REGISTRY_DIR = prev;
      rmSync(dir, { recursive: true, force: true });
    }
  }

  it("concurrent publishers never lose entries (I7)", () => {
    withShardDir(() => {
      // Simulate two TUI processes interleaving writes, as the old
      // read-modify-write raced them.
      writeRegistryEntry(entry({ tuiPid: 101, sessionId: "a", ts: NOW2 }), undefined, NOW2);
      writeRegistryEntry(entry({ tuiPid: 202, sessionId: "b", ts: NOW2 + 1 }), undefined, NOW2 + 1);
      writeRegistryEntry(entry({ tuiPid: 101, sessionId: "a", ts: NOW2 + 2 }), undefined, NOW2 + 2);
      const peers = readLivePeers(999, { now: NOW2 + 3 });
      expect(peers.map((p) => p.tuiPid).sort()).toEqual([101, 202]);
    });
  });

  it("a pid's rewrite replaces its own shard (session switch)", () => {
    withShardDir(() => {
      writeRegistryEntry(entry({ tuiPid: 101, sessionId: "old" }), undefined, NOW2);
      writeRegistryEntry(entry({ tuiPid: 101, sessionId: "new", ts: NOW2 + 5 }), undefined, NOW2 + 5);
      const peers = readLivePeers(999, { now: NOW2 + 6 });
      expect(peers).toHaveLength(1);
      expect(peers[0].sessionId).toBe("new");
    });
  });

  it("expired shards are ignored on read (TTL)", () => {
    withShardDir(() => {
      writeRegistryEntry(entry({ tuiPid: 101, sessionId: "dead", ts: NOW2 - (DEFAULT_TTL_MS + 1) }), undefined, NOW2);
      writeRegistryEntry(entry({ tuiPid: 202, sessionId: "live", ts: NOW2 }), undefined, NOW2);
      const peers = readLivePeers(999, { now: NOW2 + 1 });
      expect(peers.map((p) => p.tuiPid)).toEqual([202]);
    });
  });

  it("role round-trips through shards (I6 subagent visibility)", () => {
    withShardDir(() => {
      writeRegistryEntry(
        entry({ tuiPid: 303, sessionId: "subagent-303", role: "subagent", ts: NOW2 }),
        undefined,
        NOW2,
      );
      const peers = readLivePeers(999, { now: NOW2 + 1 });
      expect(peers).toHaveLength(1);
      expect(peers[0].role).toBe("subagent");
    });
  });

  it("removeRegistryShard drops the publisher (subagent cleanup)", () => {
    withShardDir(() => {
      writeRegistryEntry(entry({ tuiPid: 404, sessionId: "subagent-404", ts: NOW2 }), undefined, NOW2);
      expect(readLivePeers(999, { now: NOW2 + 1 })).toHaveLength(1);
      removeRegistryShard(404);
      expect(readLivePeers(999, { now: NOW2 + 1 })).toHaveLength(0);
      // idempotent
      expect(() => removeRegistryShard(404)).not.toThrow();
    });
  });

  it("self is excluded by tuiPid in shard mode too", () => {
    withShardDir(() => {
      writeRegistryEntry(entry({ tuiPid: 555, sessionId: "mine", ts: NOW2 }), undefined, NOW2);
      writeRegistryEntry(entry({ tuiPid: 666, sessionId: "peer", ts: NOW2 }), undefined, NOW2);
      expect(readLivePeers(555, { now: NOW2 + 1 }).map((p) => p.tuiPid)).toEqual([666]);
    });
  });
});

describe("gcExpiredShards", () => {
  it("removes dead-pid shards, keeps live and unparseable ones", () => {
    const dir = mkdtempSync(join(tmpdir(), "registry-gc-"));
    try {
      const now = Date.now();
      const ttl = DEFAULT_TTL_MS;
      writeFileSync(join(dir, "999.json"), JSON.stringify({ entries: [{ ts: now, tuiPid: 999, sessionId: "live", tier: "local", model: "m", ctxTokens: 1 }] }));
      writeFileSync(join(dir, "111.json"), JSON.stringify({ entries: [{ ts: now - (ttl + 1), tuiPid: 111, sessionId: "dead", tier: "local", model: "m", ctxTokens: 1 }] }));
      writeFileSync(join(dir, "777.json"), "{not json");
      gcExpiredShards(dir, now, ttl);
      expect(() => readFileSync(join(dir, "999.json"))).not.toThrow(); // live kept
      expect(() => readFileSync(join(dir, "777.json"))).not.toThrow(); // garbage kept (next pass retries)
      expect(() => readFileSync(join(dir, "111.json"))).toThrow(); // dead swept
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("is silent on a missing directory", () => {
    expect(() => gcExpiredShards("/nonexistent/registry-gc", Date.now())).not.toThrow();
  });
});
