import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { routingPresence } from "./routing-presence.ts";
import { readLivePeers, writeRegistryEntry } from "./routing-registry.ts";
import { decidePeerYield } from "../adaptive-router/peer-policy.ts";

const clock = vi.hoisted(() => ({ listeners: [] as Array<() => void> }));
vi.mock("./engine.ts", () => ({ onEngineUpdate: (fn: () => void) => {
  clock.listeners.push(fn);
  return () => { clock.listeners.splice(clock.listeners.indexOf(fn), 1); };
} }));

let directory = "";
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
  directory = mkdtempSync(join(tmpdir(), "officina-presence-"));
  vi.stubEnv("OFFICINA_ROUTING_REGISTRY_DIR", directory);
  clock.listeners.length = 0;
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllEnvs();
  rmSync(directory, { recursive: true, force: true });
});

/** Drive lifecycle and the existing telemetry heartbeat without starting timers or HTTP. */
function publisher(pid: number) {
  const handlers = new Map<string, Array<() => void>>();
  const pi = { on: (name: string, fn: () => void) => handlers.set(name, [...(handlers.get(name) ?? []), fn]) };
  const started = routingPresence(pi as unknown as ExtensionAPI, pid);
  const fire = (name: string) => { for (const fn of handlers.get(name) ?? []) fn(); };
  const publish = () => writeRegistryEntry({ ts: Date.now(), sessionStartedAt: started(),
    tuiPid: pid, sessionId: `session-${pid}`, tier: "local", model: "llamacpp/Lapis Occultus", ctxTokens: 1000 });
  return { fire, publish, started };
}

/** Advance the injected clock and exercise callbacks from the existing shared engine poller. */
function pulse(at: number): void {
  vi.setSystemTime(at);
  for (const fn of clock.listeners) fn();
}

describe("active routing presence", () => {
  it("keeps a long running prefill visible beyond TTL without changing seniority", () => {
    const h = publisher(101);
    h.fire("session_start");
    h.fire("agent_start");
    h.publish();
    for (let t = 1_010_000; t <= 1_180_000; t += 10_000) pulse(t);
    expect(readLivePeers(999)).toMatchObject([{ tuiPid: 101, sessionStartedAt: 1_000_000, ts: 1_180_000 }]);
    h.fire("agent_settled");
    pulse(1_250_001);
    expect(readLivePeers(999)).toEqual([]);
  });

  it("throttles writes, keeps the current route, and removes shutdown state", () => {
    const h = publisher(101);
    h.fire("agent_start");
    h.publish();
    pulse(1_010_000);
    pulse(1_010_700);
    expect(JSON.parse(readFileSync(join(directory, "101.json"), "utf8")).entries[0].ts).toBe(1_010_000);
    writeRegistryEntry({ ts: Date.now(), sessionStartedAt: h.started(), tuiPid: 101,
      sessionId: "session-101", tier: "cloud", model: "caduceus/new", ctxTokens: 1234 });
    pulse(1_020_000);
    expect(readLivePeers(999)[0]).toMatchObject({ tier: "cloud", model: "caduceus/new", ctxTokens: 1234 });
    h.fire("session_shutdown");
    expect(clock.listeners).toHaveLength(0);
    pulse(1_030_000);
    expect(readLivePeers(999)).toEqual([]);
    h.fire("session_start");
    expect(clock.listeners).toHaveLength(1);
  });

  it("does not carry a decision into a new session", () => {
    const h = publisher(101);
    h.fire("agent_start");
    h.publish();
    vi.setSystemTime(1_010_000);
    h.fire("session_start");
    expect(h.started()).toBe(1_010_000);
    h.fire("agent_start");
    pulse(1_020_000);
    expect(readLivePeers(999)).toEqual([]);
  });

  it("elects the same senior from both views after asymmetric heartbeat refreshes", () => {
    const older = publisher(101);
    older.fire("agent_start");
    older.publish();
    vi.setSystemTime(1_001_000);
    const newer = publisher(202);
    newer.fire("agent_start");
    newer.publish();
    pulse(1_030_000);
    const olderView = decidePeerYield({ self: { ts: older.started(), tuiPid: 101, ctxTokens: 1000, role: "main" },
      peers: readLivePeers(101), freeSlots: 0, privacy: "safe" });
    const newerView = decidePeerYield({ self: { ts: newer.started(), tuiPid: 202, ctxTokens: 1000, role: "main" },
      peers: readLivePeers(202), freeSlots: 0, privacy: "safe" });
    expect(olderView).toMatchObject({ rank: 0, demote: false });
    expect(newerView).toMatchObject({ rank: 1, demote: true });
  });
});
