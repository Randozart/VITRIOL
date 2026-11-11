import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const busy = { current: true };
const engineUp = { current: true };
const setModel = vi.fn(async (_model: unknown) => true);

vi.mock("../_shared/engine.ts", () => ({
  startEnginePolling: () => undefined,
  onEngineUpdate: () => undefined,
  getEngineSnapshot: () => ({
    up: engineUp.current,
    polled: true,
    processing: busy.current ? 1 : 0,
    deferred: 0,
    queueWaitMs: busy.current ? 2400 : 0,
    stalled: false,
    gpuLoad: 0.2,
    delta: { tps: 0, tokens: 0 },
    ingest: { tps: 0, tokens: 0 },
    cumulativeIngest: 0,
    ejected: 0,
    loaded_model: "Lapis Occultus",
    loaded_path: "",
    rpc: [],
    total: 0,
    slots: [],
    busy: busy.current ? 1 : 0,
  }),
}));

vi.mock("../_shared/events.ts", () => ({
  emitHarnessEvent: () => undefined,
  harnessEvent: () => ({}),
}));

const laneHealthState = {
  hardDown: new Set<string>(),
  backoff: new Set<string>(),
  active: new Set<string>(),
  checkedAt: 1,
};
vi.mock("../_shared/lane-health.ts", () => ({
  getLaneHealth: async () => laneHealthState,
}));

import register from "./index.ts";

function harness(contextWindows: Record<string, number> = {}) {
  const handlers = new Map<string, (e: any, c: any) => Promise<any> | any>();
  const pi = {
    on: (ev: string, h: never) => handlers.set(ev, h),
    setModel,
    registerCommand: () => undefined,
  };
  register(pi as never);
  const local = { provider: "llamacpp", id: "Lapis Occultus", contextWindow: 262144 };
  const cloud = (p: string, id: string) => ({
    provider: p,
    id,
    contextWindow: contextWindows[`${p}/${id}`] ?? 1_000_000,
  });
  const ctx = {
    model: local,
    modelRegistry: {
      find: (p: string, id: string) =>
        p === "llamacpp" ? local : id ? cloud(p, id) : undefined,
    },
    getContextUsage: () => ({ tokens: 50_000, contextWindow: 262144, percent: 19 }),
    ui: { notify: () => undefined, setStatus: () => undefined, setWidget: () => undefined },
    cwd: "/proj",
    mode: "rpc",
    hasUI: true,
    isIdle: () => true,
  };
  const start = (prompt = "do the work") =>
    handlers.get("before_agent_start")!({ messageType: "user", prompt, systemPrompt: "", systemPromptOptions: {} }, ctx);
  const trigger = (ev: string, e: unknown) => handlers.get(ev)?.(e, ctx);
  return { start, ctx, trigger };
}

describe("adaptive-router wiring", () => {
  beforeEach(() => {
    setModel.mockClear();
    busy.current = true;
    engineUp.current = true;
    process.env.OFFICINA_OVERFLOW = "1";
    process.env.OFFICINA_OVERFLOW_CLOUD = "caduceus/opencode-go/deepseek-v4.1-flash";
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = "/tmp/officina-adaptive-health-test.json";
    process.env.OFFICINA_ROUTE_LEDGER_DIR = "/tmp/officina-adaptive-ledger-test";
    process.env.OFFICINA_FREE_DAILY_TOKENS = "2000000";
    process.env.OFFICINA_FREE_MONTHLY_TOKENS = "40000000";
    // These fixtures configure SYNTHETIC explicit lanes ("caduceus/small")
    // to exercise candidate ordering — free-only (default ON) would drop
    // them, so the harness tests run with free-only disabled.
    process.env.OFFICINA_OVERFLOW_FREE_ONLY = "0";
    delete process.env.OFFICINA_NO_OVERFLOW;
  });
  afterEach(() => {
    delete process.env.OFFICINA_OVERFLOW;
    delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
    delete process.env.OFFICINA_ROUTE_LEDGER_DIR;
    delete process.env.OFFICINA_FREE_DAILY_TOKENS;
    delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
    delete process.env.OFFICINA_OVERFLOW_FREE_ONLY;
  });

  it("spills to cloud when the local engine is busy", async () => {
    const h = harness();
    await h.start();
    expect(setModel).toHaveBeenCalledTimes(1);
    expect(setModel.mock.calls[0][0]).toMatchObject({
      provider: "caduceus",
      id: "opencode-go/deepseek-v4.1-flash",
    });
  });

  it("never sends a sensitive prompt to cloud", async () => {
    const h = harness();
    await h.start("my key is sk-abcdefghijklmnopqrstuvwxyz0123456789");
    expect(setModel).not.toHaveBeenCalled();
  });

  it("fails over to cloud when the engine is DOWN", async () => {
    engineUp.current = false;
    const h = harness();
    await h.start();
    expect(setModel).toHaveBeenCalledTimes(1);
    expect(setModel.mock.calls[0][0]).toMatchObject({ provider: "caduceus" });
  });

  it("picks the first candidate that fits the context", async () => {
    process.env.OFFICINA_OVERFLOW_CLOUD = "caduceus/small,caduceus/big";
    const h = harness({ "caduceus/small": 1000, "caduceus/big": 1_000_000 });
    await h.start();
    expect(setModel.mock.calls[0][0]).toMatchObject({ provider: "caduceus", id: "big" });
  });

  it("rotates to the next candidate after a 429", async () => {
    process.env.OFFICINA_OVERFLOW_CLOUD = "caduceus/cloud-a,caduceus/cloud-b";
    const h = harness();
    await h.start();
    expect(setModel.mock.calls[0][0]).toMatchObject({ id: "cloud-a" });
    await h.trigger("after_provider_response", { status: 429 });
    await h.start();
    expect(setModel.mock.calls[1][0]).toMatchObject({ id: "cloud-b" });
  });

  it("does not fight the resilient virtual default", async () => {
    const h = harness();
    (h.ctx as any).model = { provider: "officina", id: "lapis", api: "pi-virtual" };
    await h.start();
    expect(setModel).not.toHaveBeenCalled();
  });

  it("stays local when the engine is idle", async () => {
    busy.current = false;
    const h = harness();
    await h.start();
    expect(setModel).not.toHaveBeenCalled();
  });
});
