// Lane health tests — gateway-marked endpoint health for free cloud lanes.
// classifyConnections + formatLanes are pure; getLaneHealth is tested for
// the fail-open cache behaviour with the vendor module pointed at a
// nonexistent path (import fails → previous state kept, never throws).

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  classifyConnections,
  EMPTY_HEALTH,
  formatLanes,
  getLaneHealth,
  resetLaneHealthCache,
  type GatewayConnection,
  type LaneRow,
} from "./lane-health.ts";

const conn = (over: Partial<GatewayConnection>): GatewayConnection => ({
  provider: "agentrouter",
  isActive: true,
  backoffLevel: 0,
  ...over,
});

describe("classifyConnections", () => {
  it("buckets active / backoff / hard-down owners", () => {
    const h = classifyConnections([
      conn({ provider: "opencode-go" }),
      conn({ provider: "dahl", backoffLevel: 2 }),
      conn({ provider: "agentrouter", isActive: false }),
    ]);
    expect(h.active.has("opencode-go")).toBe(true);
    expect(h.backoff.has("dahl")).toBe(true);
    expect(h.hardDown.has("agentrouter")).toBe(true);
    expect(h.checkedAt).toBeGreaterThan(0);
  });

  it("an inactive connection never lands in backoff or active", () => {
    const h = classifyConnections([conn({ isActive: false, backoffLevel: 5 })]);
    expect(h.hardDown.has("agentrouter")).toBe(true);
    expect(h.backoff.has("agentrouter")).toBe(false);
    expect(h.active.has("agentrouter")).toBe(false);
  });

  it("skips connections without a provider and tolerates garbage", () => {
    const h = classifyConnections([conn({ provider: "" }), {} as GatewayConnection, null as never]);
    expect(h.active.size).toBe(0);
    expect(h.hardDown.size).toBe(0);
    expect(h.backoff.size).toBe(0);
  });

  it("normalizes owner casing", () => {
    const h = classifyConnections([conn({ provider: "OpenCode-GO" })]);
    expect(h.active.has("opencode-go")).toBe(true);
  });
});

describe("formatLanes", () => {
  const NOW = 1_000_000_000_000;
  const row = (over: Partial<LaneRow>): LaneRow => ({
    id: "x/y",
    owner: "y",
    contextWindow: 1_000_000,
    coolingMs: 0,
    ...over,
  });

  it("orders healthy → backoff → cooling → hard-down, keeping pool order", () => {
    const rows = [
      row({ id: "opencode-go/cooling", owner: "opencode-go", coolingMs: 90_000 }),
      row({ id: "opencode-go/ok", owner: "opencode-go" }),
      row({ id: "dahl/backoff", owner: "dahl" }),
      row({ id: "agentrouter/down", owner: "agentrouter" }),
    ];
    const health = classifyConnections([
      conn({ provider: "opencode-go" }),
      conn({ provider: "dahl", backoffLevel: 1 }),
      conn({ provider: "agentrouter", isActive: false }),
    ]);
    const out = formatLanes(rows, health, NOW);
    const lines = out.split("\n");
    expect(lines[0]).toContain("opencode-go/ok");
    expect(lines[1]).toContain("dahl/backoff");
    expect(lines[1]).toContain("gateway-backoff");
    expect(lines[2]).toContain("opencode-go/cooling");
    expect(lines[2]).toContain("cooling 2m");
    expect(lines[3]).toContain("agentrouter/down");
    expect(lines[3]).toContain("gateway-down");
  });

  it("renders an empty table as an empty string (caller substitutes)", () => {
    expect(formatLanes([], EMPTY_HEALTH(), NOW)).toBe("");
  });
});

describe("getLaneHealth fail-open cache", () => {
  beforeEach(() => {
    resetLaneHealthCache();
    process.env.OFFICINA_CADUCEUS_VENDOR_API = "/nonexistent/vendor-api.mjs";
  });
  afterEach(() => {
    delete process.env.OFFICINA_CADUCEUS_VENDOR_API;
    delete process.env.OFFICINA_NO_LANE_HEALTH;
    resetLaneHealthCache();
  });

  it("returns all-active on a fetch failure (fail-open, no throw)", async () => {
    const h = await getLaneHealth();
    expect(h.hardDown.size).toBe(0);
    expect(h.backoff.size).toBe(0);
  });

  it("keeps the cached state across failed refreshes and honours the TTL", async () => {
    const first = await getLaneHealth({ ttlMs: 60_000 });
    const firstAt = first.checkedAt;
    // Second call inside the TTL must return the SAME cached object.
    const second = await getLaneHealth({ now: firstAt + 1_000, ttlMs: 60_000 });
    expect(second).toBe(first);
  });

  it("kill switch returns all-active without touching the cache", async () => {
    process.env.OFFICINA_NO_LANE_HEALTH = "1";
    const h = await getLaneHealth();
    expect(h.hardDown.size).toBe(0);
    expect(h.backoff.size).toBe(0);
  });
});
