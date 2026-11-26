import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { exactModelIds, EXACT_PROVIDER } from "../caduceus/catalog.ts";
import { exactGatewayBaseUrl, GATEWAY_BASE_URL, ROUTER_BASE_URL } from "../caduceus/base-url.ts";
import { AUTO_PROVIDER, distinctRoutes, groupModels, keyOf, pickableModels, selectRoute, type ChatModel } from "./catalog.ts";
import { eligibleRoutes, type Eligibility } from "./policy.ts";
import { recentModels, rememberModel } from "./recent.ts";
import modelCatalog, { createDiscovery } from "./index.ts";

function model(provider: string, id: string, input = 0): ChatModel {
  return {
    provider, id, name: id.split("/").at(-1)!, api: "openai-completions", baseUrl: "http://example.invalid",
    reasoning: false, input: ["text"], contextWindow: 65536, maxTokens: 4096,
    cost: { input, output: input, cacheRead: 0, cacheWrite: 0 },
  };
}

const first = model(EXACT_PROVIDER, "dahl/glm-5.3-flash");
const second = model(EXACT_PROVIDER, "freeinference/glm-5.3-flash");
const paid = model(EXACT_PROVIDER, "glm/glm-5.3-flash", 0.075);
const other = model(EXACT_PROVIDER, "glm/glm-5.3-flash-max");
const native = model("zai", "glm-5.3-flash");
const raw = (value: ChatModel) => ({
  id: value.id, owned_by: value.id.split("/")[0], root: value.id.split("/").slice(1).join("/"),
  capabilities: { tool_calling: true },
  pricing: { input: value.cost.input, output: value.cost.output },
});

describe("model catalog", () => {
  it("pins exact selections to the direct gateway and leaves custom proxies out", () => {
    expect(exactGatewayBaseUrl(ROUTER_BASE_URL)).toBe(GATEWAY_BASE_URL);
    expect(exactGatewayBaseUrl(GATEWAY_BASE_URL + "/")).toBe(GATEWAY_BASE_URL);
    expect(exactGatewayBaseUrl("http://127.0.0.1:30129/v1")).toBeUndefined();
  });

  it("recognizes provider-owned gateway identities, not shortcuts, parents or unrelated variants", () => {
    expect([...exactModelIds([
      raw(first), raw(paid), { ...raw(second), root: "different" },
      { ...raw(other), parent: paid.id },
      { id: "combo", root: "combo", owned_by: "combos" },
    ])]).toEqual([first.id, paid.id]);
    const groups = groupModels([first, second, paid, other, native]);
    expect(groups.get("glm-5.3-flash")).toEqual([first, second, paid, native]);
    expect(groups.get("glm-5.3-flash-max")).toEqual([other]);
    expect(distinctRoutes(groups.get("glm-5.3-flash")!)).toBe(true);
    expect(pickableModels([model("caduceus", first.id), first])).toEqual([first]);
  });

  it("skips parked, unknown-cost, denied and already tried routes", () => {
    const settings: Eligibility = {
      raw: [raw(first), raw(second), raw(paid)],
      prefs: null,
      budget: { paidAllowed: false, allowed: true },
      marks: { models: new Set(), owners: new Set(), pausedOwners: new Set(), invalid: new Set() },
      parked: new Map([[first.id, Date.now() + 20_000]]),
      cooldownMs: () => 0,
    };
    expect(eligibleRoutes([first, second, paid, native], settings)).toEqual([second]);
    expect(eligibleRoutes([first, second, paid, native], settings, true)).toEqual([first, second]);
    expect(selectRoute([first, second], new Set([keyOf(first)]), first)).toBe(second);
    expect(selectRoute([first], new Set([keyOf(first)]))).toBeUndefined();
    expect(eligibleRoutes([second, paid], {
      ...settings,
      prefs: { version: 1, providers: { glm: { allow_paid: true } }, models: { pinned: [], denied: [second.id] } },
      budget: { paidAllowed: true, allowed: true },
    })).toEqual([paid]);
  });

  it("coalesces discovery calls and keeps a TTL after an unavailable listing", async () => {
    vi.useFakeTimers();
    try {
      const refresh = vi.fn().mockResolvedValue({ aborted: false, errors: new Map([["zai", Error("unreachable")]]) });
      const registry = {
        getAvailable: () => [native], refresh,
        getRegisteredProviderIds: () => ["zai", "unauthenticated", "caduceus"],
        getProviderAuthStatus: (provider: string) => ({ configured: provider !== "unauthenticated" }),
      };
      const discovery = createDiscovery();
      const ctx = { modelRegistry: registry } as never;
      const one = discovery.refresh(ctx);
      expect(discovery.refresh(ctx)).toBe(one);
      await one;
      expect(refresh).toHaveBeenCalledTimes(1);
      expect(refresh.mock.calls[0]![0].providers).toEqual(["zai", "caduceus"]);
      expect(discovery.status()).toContain("cached");
      expect(discovery.refresh(ctx)).toBeUndefined();
      vi.advanceTimersByTime(300_001);
      await discovery.refresh(ctx);
      expect(refresh).toHaveBeenCalledTimes(2);
    } finally { vi.useRealTimers(); }
  });

  it("retains recent choices with route IDs across readers", () => {
    const dir = mkdtempSync(join(tmpdir(), "officina-recent-"));
    try {
      const file = join(dir, "models.jsonl");
      rememberModel(file, first);
      rememberModel(file, second);
      rememberModel(file, first);
      expect(recentModels(file)).toEqual([
        { provider: EXACT_PROVIDER, id: first.id },
        { provider: EXACT_PROVIDER, id: second.id },
      ]);
    } finally { rmSync(dir, { recursive: true, force: true }); }
  });
});

describe("picker RPC flow", () => {
  const hooks = new Map<string, Function>();
  const commands = new Map<string, (args: string, ctx: unknown) => Promise<void>>();
  const available = [first, second, paid, other, native];
  const virtuals = new Map<string, ChatModel>();
  const routers = new Map<string, Function>();
  const setModel = vi.fn().mockResolvedValue(true);
  const ui = { select: vi.fn(), notify: vi.fn(), input: vi.fn() };

  let home: string;
  beforeEach(() => {
    home = mkdtempSync(join(tmpdir(), "officina-catalog-home-"));
    vi.stubEnv("HOME", home);
  });

  afterEach(() => {
    hooks.clear(); commands.clear(); virtuals.clear(); routers.clear();
    vi.clearAllMocks();
    vi.unstubAllEnvs();
    rmSync(home, { recursive: true, force: true });
  });

  function fixture() {
    const pi = {
      setModel,
      on: (name: string, f: Function) => hooks.set(name, f),
      registerCommand: (name: string, spec: { handler: (args: string, ctx: unknown) => Promise<void> }) => commands.set(name, spec.handler),
      registerVirtualModel: (v: { id: string; name: string; provider: string; route: Function }) => {
        routers.set(v.id, v.route);
        virtuals.set(v.id, { ...model(v.provider, v.id), api: "pi-virtual", name: v.name });
      },
    };
    const ctx = {
      waitForIdle: vi.fn().mockResolvedValue(undefined), ui,
      modelRegistry: {
        getAvailable: () => available,
        find: (provider: string, id: string) => provider === AUTO_PROVIDER
          ? virtuals.get(id) : available.find((m) => m.provider === provider && m.id === id),
        getRegisteredProviderIds: () => [],
        getProviderAuthStatus: () => ({ configured: true }),
        refresh: vi.fn().mockResolvedValue({ aborted: false, errors: new Map() }),
      },
    };
    modelCatalog(pi as never);
    return ctx;
  }

  it("models opens an identity then a route, including selectable virtual auto", async () => {
    const ctx = fixture();
    ui.select
      .mockImplementationOnce(async (_title, options: string[]) => options.find((s) => s === "glm-5.3-flash · glm-5.3-flash"))
      .mockImplementationOnce(async (_title, options: string[]) => options.find((s) => s.startsWith("Auto-route")));
    await commands.get("models")!("glm-5.3-flash", ctx);
    expect(setModel).toHaveBeenCalledWith(expect.objectContaining({ provider: AUTO_PROVIDER, id: "glm-5.3-flash" }));
    expect(ui.select.mock.calls[1]![1]).not.toEqual(expect.arrayContaining([expect.stringContaining("-max")]));
  });

  it("the registered virtual route skips a failed route without using an unrelated variant", async () => {
    const ctx = fixture();
    const dir = join(home, ".vitriol", "officina");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "caduceus-models.json"), JSON.stringify({
      data: [raw(first), raw(second), raw(paid), raw(other)],
    }));
    ui.select
      .mockImplementationOnce(async (_title, options: string[]) => options.find((s) => s === "glm-5.3-flash · glm-5.3-flash"))
      .mockImplementationOnce(async (_title, options: string[]) => options.find((s) => s.startsWith("Auto-route")));
    await commands.get("models")!("", ctx);
    const route = routers.get("glm-5.3-flash")!;
    const initial = route({ reason: "user" }, ctx);
    expect(initial.model).toBe(first);
    const retry = route({ reason: "retry", failed: { model: first }, state: initial.state }, ctx);
    expect(retry.model).toBe(second);
    expect(() => route({ reason: "retry", failed: { model: second }, state: retry.state }, ctx))
      .toThrow("No eligible provider for glm-5.3-flash");
  });

  it("providers opens the actual upstream owner then selects an exact route", async () => {
    const ctx = fixture();
    ui.select
      .mockResolvedValueOnce("dahl")
      .mockImplementationOnce(async (_title, options: string[]) => options.find((s) => s.includes(first.id)));
    await commands.get("providers")!("dahl", ctx);
    expect(setModel).toHaveBeenCalledWith(first);
    expect(ui.select.mock.calls[1]![1]).not.toEqual(expect.arrayContaining([expect.stringContaining(second.id)]));
  });
});
