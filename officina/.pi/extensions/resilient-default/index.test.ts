import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { recordFailure } from "../_shared/provider-health.ts";
import { lastRouteEntry, recordRoute } from "../_shared/route-ledger.ts";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { writeRegistryEntry } from "../_shared/routing-registry.ts";

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

const engine = {
  up: true,
  busy: false,
  slots: [] as Array<{ id: number }>,
  /** Fresh admission probe answer (2026-11-20 slot contention). */
  probeBusy: false,
  /** Contention mark observed by the router. */
  contended: false,
};
vi.mock("../_shared/engine.ts", () => ({
  startEnginePolling: () => undefined,
  onEngineUpdate: () => undefined,
  // 2026-11-20 (slot contention): the probe answers from the same fixture,
  // and the contention mark is a plain flag so tests can assert BOTH inputs
  // (fresh probe vs recorded contention) independently.
  probeAdmission: async () => ({
    anyBusy: engine.probeBusy,
    slotCount: 1,
    ok: true,
  }),
  markLocalContended: () => {
    engine.contended = true;
  },
  localContended: () => engine.contended,
  CONTENTION_TTL_MS: 3000,
  clearLocalContended: () => {
    engine.contended = false;
  },
  getEngineSnapshot: () => ({
    up: engine.up,
    polled: true,
    processing: engine.busy ? 1 : 0,
    deferred: 0,
    queueWaitMs: engine.busy ? 2500 : 0,
    stalled: false,
    gpuLoad: 0.1,
    slots: engine.slots,
    slotsSeen: engine.slots.length > 0,
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
  const commands = new Map<string, { handler: (args: string, ctx: any) => unknown }>();
  const pi = {
    on: (event: string, fn: (event: any, ctx: any) => unknown) => {
      const list = handlers.get(event) ?? [];
      list.push(fn);
      handlers.set(event, list);
      return () => undefined;
    },
    registerCommand: (name: string, command: { handler: (args: string, ctx: any) => unknown }) => commands.set(name, command),
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
    ui: { setStatus: vi.fn(), setWidget: vi.fn(), notify: vi.fn() },
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
  const command = (name: string) => commands.get(name)!.handler("", ctx);
  return { run, runLane, fire, ctx, command, virtual: virtuals["lapis"], virtuals };
}

describe("resilient-default", () => {
  beforeEach(() => {
    engine.up = true;
    engine.busy = false;
    engine.slots = [];
    engine.probeBusy = false;
    engine.contended = false;
    process.env.OFFICINA_RESILIENT_DEFAULT = "1";
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model";
    // Never honour the operator's routing prefs in tests (panel I1).
    process.env.OFFICINA_CADUCEUS_PREFS_FILE = "/tmp/officina-caduceus-prefs-none.json";
    process.env.OFFICINA_PROVIDER_HEALTH_FILE = "/tmp/officina-resilient-health-test.json";
    process.env.OFFICINA_ROUTE_LEDGER_DIR = join(testRegistry, "ledger");
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

  it("yields a busy single slot to the older session despite its newer heartbeat", async () => {
    const now = Date.now();
    const date = vi.spyOn(Date, "now").mockReturnValue(now);
    const h = harness();
    engine.busy = true;
    engine.slots = [{ id: 0 }];
    date.mockReturnValue(now + 2000);
    writeRegistryEntry({ ts: now + 1500, sessionStartedAt: now - 10_000,
      tuiPid: process.pid + 1000, sessionId: "senior", tier: "local",
      model: "llamacpp/Lapis Occultus", ctxTokens: 100_000 });
    const route = await h.run("user");
    expect(route.model.provider).toBe("caduceus");
    date.mockRestore();
  });

  it("restores the physical backend badge on sticky cloud continuations", async () => {
    const h = harness();
    await h.run("continuation", "continue", { previous: { model: previous } });
    expect(h.ctx.ui.setStatus).toHaveBeenCalledWith("overflow", expect.stringContaining("previous-model"));
    await h.fire("message_end", { message: { role: "assistant", stopReason: "stop" } });
    expect(lastRouteEntry()).toMatchObject({ tuiPid: process.pid, sessionId: `tui-${process.pid}` });
  });

  it("explains this session's pending/completed route instead of another session's latest record", async () => {
    const h = harness();
    await h.run("user");
    await h.command("route-why");
    expect(h.ctx.ui.notify).toHaveBeenLastCalledWith(expect.stringContaining("in flight"), "info");
    await h.fire("message_end", { message: { role: "assistant", stopReason: "stop" } });
    recordRoute({ ts: new Date(Date.now() + 100).toISOString(), sessionId: "someone-else", reason: "user", tier: "cloud", model: "other-session" });
    await h.command("route-why");
    expect(h.ctx.ui.notify).toHaveBeenLastCalledWith(expect.stringContaining("llamacpp/Lapis Occultus"), "info");
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

  it("returns the fresh replacement after cooling the originally selected cloud lane", async () => {
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK = "caduceus/test-model,caduceus/second-model";
    process.env.OFFICINA_GATEWAY_HEALTH_FILE = join(testRegistry, "lapis-failure-marks.json");
    const h = harness();
    const result = await h.run("retry", "continue", {
      failed: { model: cloud, message: { errorMessage: '429 model_cooldown {"reset_seconds":120}' } },
    });
    expect(result.model).toBe(cloud2);
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
    process.env.OFFICINA_ROUTE_LEDGER_DIR = join(testRegistry, "ledger");
    process.env.OFFICINA_ROUTING_PROFILE_FILE = "/tmp/officina-nimbus-profile-test";
    process.env.OFFICINA_GATEWAY_HEALTH_FILE = join(testRegistry, "nimbus-failure-marks.json");
    rmSync(process.env.OFFICINA_GATEWAY_HEALTH_FILE, { force: true });
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
    delete process.env.OFFICINA_GATEWAY_HEALTH_FILE;
    delete process.env.OFFICINA_CLOUD_RETRIES;
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

  it("handles the bare credential cooldown on retry without a response callback", async () => {
    const h = harness();
    const run = h.runLane("nimbus");
    const first = await run("user");
    const replacement = await run("retry", "continue", { state: first.state,
      failed: { model: first.model, message: { errorMessage: "All credentials for model aion-labs/aion-3.0 are cooling down" } },
    });
    expect(replacement.model).toBe(cloud2);
    expect(replacement.state.cloudRetries).toBe(2);
  });

  it("refuses to resurrect cooling lanes when every cloud lane is cooling", async () => {
    recordFailure("caduceus", cloud.id, 429);
    recordFailure("caduceus", cloud2.id, 429);
    await expect(harness().runLane("nimbus")("user")).rejects.toThrow(/candidates cooling/);
  });

  it("stops at the Nimbus retry budget without selecting local", async () => {
    process.env.OFFICINA_CLOUD_RETRIES = "1";
    const h = harness();
    await expect(h.runLane("nimbus")("retry", "continue", { state: { cloudRetries: 1, turnsOnTier: 1 },
      failed: { model: cloud, message: { errorMessage: "429 model_cooldown" } },
    })).rejects.toThrow(/Nimbus: cloud retry budget exhausted/);
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
    process.env.OFFICINA_ROUTE_LEDGER_DIR = join(testRegistry, "ledger");
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

// Synthetic routes must never publish into the operator's live registry.
let testRegistry = "";
let originalRegistry: string | undefined;
beforeEach(() => {
  originalRegistry = process.env.OFFICINA_ROUTING_REGISTRY_DIR;
  testRegistry = mkdtempSync(join(tmpdir(), "officina-router-registry-"));
  process.env.OFFICINA_ROUTING_REGISTRY_DIR = testRegistry;
});
afterEach(() => {
  if (originalRegistry === undefined) delete process.env.OFFICINA_ROUTING_REGISTRY_DIR;
  else process.env.OFFICINA_ROUTING_REGISTRY_DIR = originalRegistry;
  rmSync(testRegistry, { recursive: true, force: true });
});

// ── Slot contention (2026-11-20) ───────────────────────────────────────────
// The operator's report: Lapis routed to the one-slot local engine while that
// slot was held by another session. The 700 ms engine snapshot said idle; the
// FRESH probe says busy. Routing assertions only — the ledger is written at
// message_end, so it is exercised explicitly where it matters (I5).
describe("resilient-default slot contention", () => {
  beforeEach(() => {
    process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK =
      "caduceus/test-model,caduceus/second-model";
    process.env.OFFICINA_FREE_DAILY_TOKENS = "2000000";
    process.env.OFFICINA_FREE_MONTHLY_TOKENS = "40000000";
  });
  afterEach(() => {
    delete process.env.OFFICINA_OVERFLOW_CLOUD_FALLBACK;
    delete process.env.OFFICINA_FREE_DAILY_TOKENS;
    delete process.env.OFFICINA_FREE_MONTHLY_TOKENS;
  });

  // Declaration order matters: this case must run with a clean local tier.
  it("stays local when the probe reports the slot free (parity, I1/I6)", async () => {
    const h = harness();
    const route = await h.run("user");
    expect(route.model.provider).toBe("llamacpp");
  });

  it("routes to cloud when the fresh probe sees the slot busy", async () => {
    engine.probeBusy = true;
    const h = harness();
    const route = await h.run("user");
    expect(route.model.provider).toBe("caduceus");
  });

  it("records localState 'contended' when the probe observed it (I5)", async () => {
    engine.probeBusy = true;
    const h = harness();
    await h.run("user");
    // The ledger row is written at message_end, not at route time.
    await h.fire("message_end", {
      message: { role: "assistant", stopReason: "end_turn", usage: {} },
    });
    expect(lastRouteEntry()?.localState).toBe("contended");
  });

  it("marks contention on a local 4xx response (was silently ignored)", async () => {
    // Force a LOCAL route: with no cloud budget the router returns home even
    // from a cloud tier, so the local failure path is the one under test.
    process.env.OFFICINA_FREE_DAILY_TOKENS = "0";
    const h = harness();
    const route = await h.run("user");
    expect(route.model.provider).toBe("llamacpp");
    await h.fire("after_provider_response", { status: 503, headers: {} });
    expect(engine.contended).toBe(true);
    expect(h.ctx.ui.notify).toHaveBeenCalledWith(
      expect.stringContaining("contended"),
      "info",
    );
  });
});
