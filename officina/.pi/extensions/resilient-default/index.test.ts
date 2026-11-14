import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { recordFailure } from "../_shared/provider-health.ts";
import { lastRouteEntry } from "../_shared/route-ledger.ts";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Gateway lane health — static marking the tests can mutate per case.
const laneHealthState = {
  hardDown: new Set<string>(),
  backoff: new Set<string>(),
  active: new Set<string>(),
  checkedAt: 1,
};
vi.mock("../_shared/lane-health.ts", () => ({
  getLaneHealth: async () => laneHealthState,
  formatLanes: (rows: Array<{ id: string }>) => rows.map((r) => r.id).join("\n"),
  EMPTY_HEALTH: () => ({ hardDown: new Set(), backoff: new Set(), active: new Set(), checkedAt: 0 }),
}));

const engine = { up: true, busy: false };
vi.mock("../_shared/engine.ts", () => ({
  startEnginePolling: () => undefined,
  onEngineUpdate: () => undefined,
  getEngineSnapshot: () => ({
    up: engine.up,
    polled: true,
    processing: engine.busy ? 1 : 0,
    deferred: 0,
    queueWaitMs: engine.busy ? 2500 : 0,
    stalled: false,
    gpuLoad: 0.1,
  }),
}));

vi.mock("../_shared/events.ts", () => ({
  emitHarnessEvent: () => undefined,
  harnessEvent: () => ({}),
}));

import register from "./index.ts";

const local = { provider: "llamacpp", id: "Lapis Occultus", api: "llamacpp", contextWindow: 262144 };
const cloud = { provider: "caduceus", id: "test-model", api: "openai-completions", contextWindow: 1000000 };
const cloud2 = { provider: "caduceus", id: "second-model", api: "openai-completions", contextWindow: 500000 };
const previous = { provider: "caduceus", id: "previous-model", api: "openai-completions", contextWindow: 1000000 };

function harness(opts: { tokens?: number; localId?: string | null; noLocal?: boolean } = {}) {
  const virtuals: Record<string, any> = {};
  const handlers = new Map<string, Array<(event: any, ctx: any) => unknown>>();
  const pi = {
    on: (event: string, fn: (event: any, ctx: any) => unknown) => {
      const list = handlers.get(event) ?? [];
      list.push(fn);
      handlers.set(event, list);
      return () => undefined;
    },
    registerCommand: () => undefined,
    registerVirtualModel: (m: any) => {
      virtuals[m.id] = m;
    },
  };
  register(pi as never);
  if (!virtuals["lapis"]) throw new Error("lapis virtual model not registered");
  const staleLocal = { ...local, id: "Qwen3.8-27B-Q3_K_M.gguf" };
  const localList = opts.noLocal ? [] : opts.localId === null ? [staleLocal] : [local];
  const ctx = {
    modelRegistry: {
      getAll: () => (opts.noLocal ? [] : localList),
      getAvailable: () => localList,
      find: (provider: string, id: string) => {
        if (provider === "llamacpp") return opts.noLocal || opts.localId === null ? undefined : local;
        if (id === "test-model") return cloud;
        if (id === "second-model") return cloud2;
        if (id === "previous-model") return previous;
        return undefined;
      },
      hasConfiguredAuth: () => true,
    },
    getContextUsage: () => ({
      tokens: opts.tokens ?? 50_000,
      contextWindow: 262144,
      percent: 19,
    }),
    ui: { setStatus: () => undefined, setWidget: () => undefined },
  };
  const run = (reason: string, prompt = "do work", extra: Record<string, unknown> = {}) =>
    virtuals["lapis"].route(
      {
        reason,
        model: { provider: "officina", id: "lapis", api: "pi-virtual" },
        thinkingLevel: "off",
        messages: [{ role: "user", content: prompt }],
        state: undefined,
        ...extra,
      },
      ctx,
    );
  const runLane = (id: string) => (reason: string, prompt = "do work", extra: Record<string, unknown> = {}) =>
    virtuals[id].route(
      {
        reason,
        model: { provider: "officina", id, api: "pi-virtual" },
        thinkingLevel: "off",
        messages: [{ role: "user", content: prompt }],
        state: undefined,
        ...extra,
      },
      ctx,
    );
  const fire = async (event: string, payload: any): Promise<void> => {
    for (const fn of handlers.get(event) ?? []) await fn(payload, ctx);
  };
  return { run, runLane, fire, virtual: virtuals["lapis"], virtuals };
}

describe("resilient-default", () => {
  beforeEach(() => {
    engine.up = true;
    engine.busy = false;
    process.env.OFFICINA_RESILIENT_DEFAULT = "1";
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model";
    // Never honour the operator's routing prefs in tests (panel I1).
    process.env.OFFICINA_CADUCEUS_PREFS_FILE = "/tmp/officina-caduceus-prefs-none.json";
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = "/tmp/officina-resilient-health-test.json";
    process.env.OFFICINA_ROUTE_LEDGER_DIR = "/tmp/officina-resilient-ledger-test";
    process.env.OFFICINA_ROUTING_PROFILE_FILE = "/tmp/officina-resilient-profile-test";
    process.env.OFFICINA_FREE_DAILY_TOKENS = "2000000";
    // Deterministic picks: never consult the operator's live peer registry.
    process.env.OFFICINA_NO_LANE_SPREAD = "1";
    process.env.OFFICINA_NO_GATEWAY_MARKS = "1";
    // Never touch the operator's live provider-health file.
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = join(tmpdir(), "officina-resilient-health-test.json");
    rmSync(process.env.OFFICINA_PROVIDER_HEALTH_FILE, { force: true });
    process.env.OFFICINA_FREE_MONTHLY_TOKENS = "40000000";
  });
  afterEach(() => {
    delete process.env.OFFICINA_RESILIENT_DEFAULT;
    delete process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK;
    delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
    delete process.env.OFFICINA_ROUTE_LEDGER_DIR;
    delete process.env.OFFICINA_NO_LANE_SPREAD;
    delete process.env.OFFICINA_NO_GATEWAY_MARKS;
    delete process.env.OFFICINA_GATEWAY_HEALTH_FILE;
    delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
    delete process.env.OFFICINA_CLOUD_RETRIES;
    delete process.env.OFFICINA_ROUTING_PROFILE_FILE;
    delete process.env.OFFICINA_FREE_DAILY_TOKENS;
    delete process.env.OFFICINA_NO_LANE_SPREAD;
    delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
  });

  it("registers the keyless officina/lapis virtual model", () => {
    const h = harness();
    expect(h.virtual).toMatchObject({ provider: "officina", id: "lapis", name: "Lapis Occultus" });
  });

  it("routes to local when the engine is up and idle", async () => {
    const h = harness();
    const route = await h.run("user");
    expect(route.model).toMatchObject({ provider: "llamacpp", id: "Lapis Occultus" });
  });

  it("routes to cloud when the engine is down", async () => {
    engine.up = false;
    const h = harness();
    const route = await h.run("user");
    expect(route.model).toMatchObject({ provider: "caduceus", id: "test-model" });
  });

  it("sticks to the previous physical model on continuation", async () => {
    const h = harness();
    const route = await h.run("continuation", "continue", { previous: { model: previous } });
    expect(route.model).toBe(previous);
    expect(route.state).toBeUndefined();
  });

  it("routes local through a stale registered id when the alias was never probed", async () => {
    engine.up = true;
    engine.busy = false;
    const h = harness({ localId: null });
    const route = await h.run("user");
    expect(route.model.provider).toBe("llamacpp");
    expect(route.model.id).toBe("Qwen3.8-27B-Q3_K_M.gguf");
  });

  it("skips a lane the gateway marked invalid (model_not_found)", async () => {
    engine.up = true;
    engine.busy = false;
    const dir = mkdtempSync(join(tmpdir(), "gw-invalid-"));
    const path = join(dir, "gateway-health.json");
    const until = Date.now() + 3600_000;
    writeFileSync(
      path,
      JSON.stringify({
        version: 1,
        ts: Date.now(),
        models: {},
        owners: {},
        invalid: { "test-model": { until, reason: "not offered" } },
      }),
    );
    process.env.OFFICINA_NO_GATEWAY_MARKS = "0";
    process.env.OFFICINA_GATEWAY_HEALTH_FILE = path;
    const h = harness();
    const route = await h.run("user");
    expect(route.model.provider).toBe("llamacpp"); // first candidate skipped
    rmSync(dir, { recursive: true, force: true });
  });

  it("routes local after a context-overflow retry (no fit lottery)", async () => {
    engine.up = true;
    engine.busy = false;
    const h = harness();
    const overflow = {
      model: { provider: "caduceus", id: "test-model" },
      message: {
        errorMessage:
          "[400]: This model's maximum context length is 180000 tokens. However, you requested 8192 output tokens and your prompt contains at least 171809 input tokens (parameter=input_tokens, value=171809)",
      },
    };
    const route = await h.run("retry", "continue", { failed: overflow });
    expect(route.model.provider).toBe("llamacpp");
  });

  it("skips a paused owner (400 key circuit) and routes local", async () => {
    engine.up = true;
    engine.busy = false;
    const dir = mkdtempSync(join(tmpdir(), "gw-paused-"));
    const path = join(dir, "gateway-health.json");
    writeFileSync(
      path,
      JSON.stringify({
        version: 1,
        ts: Date.now(),
        models: {},
        owners: {},
        invalid: {},
        pauses: { "test-model": { until: Date.now() + 5 * 60_000, safeBytes: 56 * 1024 } },
      }),
    );
    process.env.OFFICINA_NO_GATEWAY_MARKS = "0";
    process.env.OFFICINA_GATEWAY_HEALTH_FILE = path;
    const h = harness();
    const route = await h.run("user");
    expect(route.model.provider).toBe("llamacpp");
    rmSync(dir, { recursive: true, force: true });
  });

  it("respects the cloud retry budget (0 => local immediately)", async () => {
    engine.up = true;
    engine.busy = false;
    process.env.OFFICINA_CLOUD_RETRIES = "0";
    const h = harness();
    const route = await h.run("retry", "continue", {
      failed: { model: { provider: "caduceus", id: "test-model" }, message: { errorMessage: "429" } },
    });
    expect(route.model.provider).toBe("llamacpp");
    delete process.env.OFFICINA_CLOUD_RETRIES;
  });

  it("prefers a lane with a fresh successful probe", async () => {
    engine.up = true;
    engine.busy = false;
    const dir = mkdtempSync(join(tmpdir(), "gw-verified-"));
    const path = join(dir, "gateway-health.json");
    writeFileSync(
      path,
      JSON.stringify({
        version: 1,
        ts: Date.now(),
        models: {},
        owners: {},
        invalid: {},
        probes: {
          "second-model": {
            kind: "ok",
            okAt: Date.now() - 1_000,
            failCount: 0,
            nextProbeAt: Date.now() + 600_000,
          },
        },
      }),
    );
    process.env.OFFICINA_NO_GATEWAY_MARKS = "0";
    process.env.OFFICINA_GATEWAY_HEALTH_FILE = path;
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model,caduceus/second-model";
    const h = harness({ noLocal: true });
    const route = await h.run("user");
    expect(route.model.id).toBe("second-model");
    rmSync(dir, { recursive: true, force: true });
  });

  it("records admission skip reasons in the route ledger", async () => {
    engine.up = true;
    engine.busy = false;
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model,caduceus/second-model";
    const healthDir = mkdtempSync(join(tmpdir(), "gw-ledger-health-"));
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = join(healthDir, "health.json");
    recordFailure("caduceus", "test-model", 429, { retryAfterMs: 120_000 });
    const h = harness({ noLocal: true });
    await h.run("user");
    await h.fire("message_end", {
      message: { role: "assistant", stopReason: "end_turn", usage: { input: 10, output: 5 } },
    });
    const entry = lastRouteEntry();
    expect(entry?.model).toBe("second-model");
    expect(entry?.candidatesSkipped?.some((s: string) => s.includes("test-model"))).toBe(true);
    rmSync(healthDir, { recursive: true, force: true });
    delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
  });

  it("names the real gate when nothing can serve", async () => {
    engine.up = false;
    process.env.OFFICINA_FREE_DAILY_TOKENS = "0";
    const h = harness({ noLocal: true });
    await expect(h.run("user")).rejects.toThrow(/no local model configured/);
    await expect(h.run("user")).rejects.toThrow(/cloud budget exhausted/);
  });

  it("fails over to cloud on a local retry", async () => {
    const h = harness();
    const route = await h.run("retry", "retry", { failed: { model: local, message: {} } });
    expect(route.model).toMatchObject({ provider: "caduceus", id: "test-model" });
  });
});

// Nimbus — the cloud-only lane (2026-11-11): local is NEVER selected, in
// any engine state. Candidates are the same verified-free pool as lapis's
// cloud tier.
describe("nimbus (cloud-only lane)", () => {
  beforeEach(() => {
    engine.up = true;
    engine.busy = false;
    process.env.OFFICINA_RESILIENT_DEFAULT = "1";
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model,caduceus/second-model";
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = "/tmp/officina-nimbus-health-test.json";
    process.env.OFFICINA_ROUTE_LEDGER_DIR = "/tmp/officina-nimbus-ledger-test";
    process.env.OFFICINA_ROUTING_PROFILE_FILE = "/tmp/officina-nimbus-profile-test";
    process.env.OFFICINA_FREE_DAILY_TOKENS = "2000000";
    process.env.OFFICINA_FREE_MONTHLY_TOKENS = "40000000";
    process.env.OFFICINA_NO_LANE_SPREAD = "1";
    // Provider-health persists across runs — a 429 from an earlier run
    // must not cool test-model into this one.
    rmSync("/tmp/officina-nimbus-health-test.json", { force: true });
    laneHealthState.hardDown = new Set();
    laneHealthState.backoff = new Set();
  });
  afterEach(() => {
    delete process.env.OFFICINA_RESILIENT_DEFAULT;
    delete process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK;
    delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
    delete process.env.OFFICINA_ROUTE_LEDGER_DIR;
    delete process.env.OFFICINA_ROUTING_PROFILE_FILE;
    delete process.env.OFFICINA_FREE_DAILY_TOKENS;
    delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
  });

  it("registers officina/nimbus alongside lapis", () => {
    const h = harness();
    expect(h.virtuals["nimbus"]).toMatchObject({ provider: "officina", id: "nimbus" });
    expect(h.virtuals["lapis"]).toMatchObject({ provider: "officina", id: "lapis" });
  });

  it("routes cloud on an idle local engine — the whole point of the lane", async () => {
    const h = harness();
    const route = await h.runLane("nimbus")("user");
    expect(route.model).toMatchObject({ provider: "caduceus", id: "test-model" });
  });

  it("routes cloud with the engine DOWN too (no local anywhere)", async () => {
    engine.up = false;
    const h = harness();
    const route = await h.runLane("nimbus")("user");
    expect(route.model).toMatchObject({ provider: "caduceus" });
  });

  it("sticks to the previous physical model on continuation", async () => {
    const h = harness();
    const route = await h.runLane("nimbus")("continuation", "continue", { previous: { model: previous } });
    expect(route.model).toBe(previous);
  });

  it("rotates to the next free lane after a 429 cools the first", async () => {
    const h = harness();
    const nimbus = h.runLane("nimbus");
    const first = await nimbus("user");
    expect(first.model).toMatchObject({ id: "test-model" });
    h.virtuals["nimbus"]; // lane object; cooling happens via the provider-health module
    const { recordFailure } = await import("../_shared/provider-health.ts");
    recordFailure("caduceus", "test-model", 429);
    const second = await nimbus("user");
    expect(second.model).toMatchObject({ id: "second-model" });
  });

  it("dies loudly when no free lane fits — never falls back to local", async () => {
    const h = harness({ tokens: 2_000_000 }); // fits no mock lane
    await expect(h.runLane("nimbus")("user")).rejects.toThrow(/no free cloud lane/);
  });
});

describe("nimbus lane health (gateway-marked endpoints)", () => {
  beforeEach(() => {
    engine.up = true;
    engine.busy = false;
    process.env.OFFICINA_RESILIENT_DEFAULT = "1";
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model,caduceus/second-model";
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = "/tmp/officina-nimbus-health-test.json";
    process.env.OFFICINA_ROUTE_LEDGER_DIR = "/tmp/officina-nimbus-ledger-test";
    process.env.OFFICINA_ROUTING_PROFILE_FILE = "/tmp/officina-nimbus-profile-test";
    process.env.OFFICINA_FREE_DAILY_TOKENS = "2000000";
    process.env.OFFICINA_FREE_MONTHLY_TOKENS = "40000000";
    rmSync("/tmp/officina-nimbus-health-test.json", { force: true });
    laneHealthState.hardDown = new Set();
    laneHealthState.backoff = new Set();
  });
  afterEach(() => {
    delete process.env.OFFICINA_RESILIENT_DEFAULT;
    delete process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK;
    delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
    delete process.env.OFFICINA_ROUTE_LEDGER_DIR;
    delete process.env.OFFICINA_ROUTING_PROFILE_FILE;
    delete process.env.OFFICINA_FREE_DAILY_TOKENS;
    delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
  });

  it("skips a gateway-hard-down lane entirely — pre-flight, no wasted turn", async () => {
    laneHealthState.hardDown = new Set(["test-model"]);
    const h = harness();
    const route = await h.runLane("nimbus")("user");
    expect(route.model).toMatchObject({ provider: "caduceus", id: "second-model" });
  });

  it("demotes a gateway-backoff lane behind healthy ones", async () => {
    laneHealthState.backoff = new Set(["test-model"]);
    const h = harness();
    // test-model is still selectable when nothing healthy fits/exists
    // (demoted, not excluded) — with second-model healthy, it wins.
    const route = await h.runLane("nimbus")("user");
    expect(route.model).toMatchObject({ id: "second-model" });
  });
});
