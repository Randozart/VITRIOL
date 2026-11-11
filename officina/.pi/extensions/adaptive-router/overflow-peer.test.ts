// P3 integration: the peer arm inside decideTier — parity + demotion contract.
//
// I1 (the load-bearing one): when peer context is absent, decisions must be
// byte-identical to the pre-P3 policy. The golden matrix below pins a spread
// of pre-P3 outcomes; if the peer arm ever changes a single-session decision
// this file fails.

import { describe, expect, it } from "vitest";
import {
  configFromEnv,
  decideTier,
  type DecideInput,
  type LoadSample,
  type RouterState,
} from "./overflow.ts";
import type { PeerView, SelfView } from "./peer-policy.ts";

const T0 = 1_000_000_000_000;
const cfg = configFromEnv({});

const idle: LoadSample = {
  processing: 0,
  deferred: 0,
  queueWaitMs: 0,
  stalled: false,
  gpuLoad: null,
};
const busyLoad: LoadSample = {
  processing: 1,
  deferred: 0,
  queueWaitMs: 0,
  stalled: false,
  gpuLoad: null,
};

function state(over: Partial<RouterState> = {}): RouterState {
  return { tier: "local", switches: 0, turnsOnTier: 1, cloudTokens: 0, ...over };
}

function base(over: Partial<DecideInput> = {}): DecideInput {
  return {
    state: state(),
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

const me: SelfView = { ts: T0 + 500, tuiPid: 100, ctxTokens: 8_000, role: "main" };
const seniorPeer: PeerView = {
  ts: T0,
  tuiPid: 200,
  sessionId: "senior",
  tier: "local",
  model: "llamacpp/Lapis Occultus",
  ctxTokens: 120_000,
};

describe("I1 parity: no peer context ⇒ pre-P3 decisions", () => {
  const matrix: Array<{ name: string; input: DecideInput; tier: "local" | "cloud" }> = [
    { name: "idle local stays local", input: base(), tier: "local" },
    { name: "busy local spills", input: base({ load: busyLoad }), tier: "cloud" },
    {
      name: "busy local with no cloud stays local",
      input: base({ load: busyLoad, cloudFits: false }),
      tier: "local",
    },
    {
      name: "confidential never leaves local",
      input: base({ load: busyLoad, privacy: "confidential" }),
      tier: "local",
    },
    {
      name: "engine down fails over",
      input: base({ localDown: true }),
      tier: "cloud",
    },
    {
      name: "switch cap reached keeps cloud",
      input: base({ state: state({ tier: "cloud", switches: 2, turnsOnTier: 5 }) }),
      tier: "cloud",
    },
  ];

  for (const c of matrix) {
    it(c.name, () => {
      const withPeersAbsent = decideTier(c.input, cfg);
      // Same input plus explicitly-empty peer context (the wired shape) must
      // agree with the absent shape.
      const withEmptyPeers = decideTier(
        { ...c.input, self: me, peers: [], freeSlots: 1 },
        cfg,
      );
      expect(withPeersAbsent.tier).toBe(c.tier);
      expect(withEmptyPeers.tier).toBe(c.tier);
      expect(withEmptyPeers.reason).toBe(withPeersAbsent.reason);
    });
  }
});

describe("P3 demotion under full slots", () => {
  it("junior session yields when no slot is free", () => {
    const d = decideTier(
      base({ self: me, peers: [seniorPeer], freeSlots: 0 }),
      cfg,
    );
    expect(d.tier).toBe("cloud");
    expect(d.peerDemoted).toBe(true);
    expect(d.reason).toMatch(/rank|slot/i);
  });

  it("the senior session keeps the stone", () => {
    const seniorSelf: SelfView = { ...me, ts: T0, ctxTokens: 120_000 };
    const d = decideTier(
      base({ self: seniorSelf, peers: [{ ...seniorPeer, ts: T0 + 500 }], freeSlots: 0 }),
      cfg,
    );
    expect(d.tier).toBe("local");
    expect(d.peerDemoted).toBeUndefined();
  });

  it("a freed slot ends the demotion", () => {
    const d = decideTier(base({ self: me, peers: [seniorPeer], freeSlots: 1 }), cfg);
    expect(d.tier).toBe("local");
    // With capacity available the peer arm has nothing to arbitrate; the
    // normal idle path decides (no demotion, no cap charge).
    expect(d.peerDemoted).toBeUndefined();
  });

  it("never demotes without a viable cloud candidate", () => {
    const d = decideTier(
      base({ self: me, peers: [seniorPeer], freeSlots: 0, cloudFits: false }),
      cfg,
    );
    expect(d.tier).toBe("local");
  });

  it("never demotes with the cloud budget exhausted", () => {
    const d = decideTier(
      base({ self: me, peers: [seniorPeer], freeSlots: 0, budgetLeft: false }),
      cfg,
    );
    expect(d.tier).toBe("local");
  });

  it("privacy beats peer pressure (I2)", () => {
    const d = decideTier(
      base({ self: me, peers: [seniorPeer], freeSlots: 0, privacy: "sensitive" }),
      cfg,
    );
    expect(d.tier).toBe("local");
  });

  it("failover outranks peer arbitration (I3)", () => {
    const d = decideTier(
      base({ self: me, peers: [seniorPeer], freeSlots: 0, localDown: true }),
      cfg,
    );
    expect(d.tier).toBe("cloud");
    expect(d.peerDemoted).toBeUndefined(); // emergency, not peer arbitration
    expect(d.reason).toMatch(/down/i);
  });
});
