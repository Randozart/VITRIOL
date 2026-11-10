import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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
const previous = { provider: "caduceus", id: "previous-model", api: "openai-completions", contextWindow: 1000000 };

function harness() {
  let virtual: any = null;
  const pi = {
    on: () => undefined,
    registerCommand: () => undefined,
    registerVirtualModel: (m: any) => {
      virtual = m;
    },
  };
  register(pi as never);
  if (!virtual) throw new Error("virtual model not registered");
  const ctx = {
    modelRegistry: {
      find: (provider: string, id: string) => {
        if (provider === "llamacpp") return local;
        if (id === "test-model") return cloud;
        if (id === "previous-model") return previous;
        return undefined;
      },
      hasConfiguredAuth: () => true,
    },
    getContextUsage: () => ({ tokens: 50_000, contextWindow: 262144, percent: 19 }),
    ui: { setStatus: () => undefined, setWidget: () => undefined },
  };
  const run = (reason: string, prompt = "do work", extra: Record<string, unknown> = {}) =>
    virtual.route(
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
  return { run, virtual };
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
