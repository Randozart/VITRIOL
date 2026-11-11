import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { rmSync } from "node:fs";

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

function harness(opts: { tokens?: number } = {}) {
  const virtuals: Record<string, any> = {};
  const pi = {
    on: () => undefined,
    registerCommand: () => undefined,
    registerVirtualModel: (m: any) => {
      virtuals[m.id] = m;
    },
  };
  register(pi as never);
  if (!virtuals["lapis"]) throw new Error("lapis virtual model not registered");
  const ctx = {
    modelRegistry: {
      find: (provider: string, id: string) => {
        if (provider === "llamacpp") return local;
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
  return { run, runLane, virtual: virtuals["lapis"], virtuals };
}

describe("resilient-default", () => {
  beforeEach(() => {
    engine.up = true;
    engine.busy = false;
    process.env.OFFICINA_RESILIENT_DEFAULT = "1";
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model";
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = "/tmp/officina-resilient-health-test.json";
    process.env.OFFICINA_ROUTE_LEDGER_DIR = "/tmp/officina-resilient-ledger-test";
    process.env.OFFICINA_ROUTING_PROFILE_FILE = "/tmp/officina-resilient-profile-test";
    process.env.OFFICINA_FREE_DAILY_TOKENS = "2000000";
    process.env.OFFICINA_FREE_MONTHLY_TOKENS = "40000000";
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
    // Provider-health persists across runs — a 429 from an earlier run
    // must not cool test-model into this one.
    rmSync("/tmp/officina-nimbus-health-test.json", { force: true });
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
