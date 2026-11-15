// Lane health tests — gateway-marked endpoint health for free cloud lanes.
// classifyConnections + formatLanes are pure; getLaneHealth is tested for
// the fail-open cache behaviour with the vendor module pointed at a
// nonexistent path (import fails → previous state kept, never throws).

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  applyModelHealth,
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

  it("loads a vendor module from a path containing spaces and URL-special characters", async () => {
    const dir = mkdtempSync(join(tmpdir(), "lane health #"));
    const file = join(dir, "vendor api.mjs");
    writeFileSync(file, "export async function apiFetch() { return {ok:true,json:async()=>[{provider:'agentrouter',isActive:false}]}; }");
    process.env.OFFICINA_CADUCEUS_VENDOR_API = file;
    try {
      // Exercise Node's actual external-module loader; Vite's test resolver does
      // not resolve file URLs containing encoded spaces or fragments.
      const resultFile = join(dir, "result.json");
      const moduleUrl = pathToFileURL(join(__dirname, "lane-health.ts")).href;
      const code = `import {getLaneHealth} from ${JSON.stringify(moduleUrl)};
        const health = await getLaneHealth();
        (await import('node:fs')).writeFileSync(process.argv[1], JSON.stringify([...health.hardDown]));`;
      const child = spawn(process.execPath, ["--experimental-strip-types", "--input-type=module", "-e", code, resultFile], { stdio: "ignore" });
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => { child.kill("SIGKILL"); reject(new Error("native vendor import timed out")); }, 2000);
        child.once("error", (error) => { clearTimeout(timer); reject(error); });
        child.once("exit", (code, signal) => {
          clearTimeout(timer);
          if (code === 0) resolve(); else reject(new Error(`native import exited: code=${code} signal=${signal}`));
        });
      });
      expect(JSON.parse(readFileSync(resultFile, "utf8"))).toContain("agentrouter");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
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

describe("applyModelHealth", () => {
  it("demotes circuit-open/rate-limited providers to backoff, never hardDown", () => {
    const health = EMPTY_HEALTH();
    health.active.add("freeinference");
    health.active.add("dahl");
    applyModelHealth(health, [
      { provider: "freeinference", canExecute: false, circuitBreakerState: "OPEN" },
      { provider: "dahl", circuitBreakerState: "CLOSED", connections: [{ state: "RATE_LIMITED" }] },
      { provider: "agentrouter", circuitBreakerState: "CLOSED", connections: [{ state: "HEALTHY" }] },
    ]);
    expect(health.backoff.has("freeinference")).toBe(true);
    expect(health.backoff.has("dahl")).toBe(true);
    expect(health.active.has("freeinference")).toBe(false);
    expect(health.active.has("dahl")).toBe(false);
    expect(health.hardDown.size).toBe(0);
    expect(health.backoff.has("agentrouter")).toBe(false);
  });
});
