import { afterEach, describe, expect, it, vi } from "vitest";
import { createJiti } from "jiti";
import { normalizeContext } from "@earendil-works/pi-ai";
import { localCacheOwner, registerCacheOwners } from "./owner.ts";
import { streamLocalWithProgress } from "./progress.ts";

vi.mock("../_shared/events.ts", () => ({ emitHarnessEvent: () => {}, harnessEvent: () => ({}) }));
const cleanups: Array<() => void> = [];
afterEach(() => { for (const close of cleanups.splice(0)) close(); vi.unstubAllEnvs(); });

function harness(register = registerCacheOwners) {
  const entries: any[] = [];
  const handlers = new Map<string, (event: unknown, ctx: any) => unknown>();
  const pi = { on: (name: string, handler: (event: unknown, ctx: any) => unknown) => handlers.set(name, handler),
    appendEntry: vi.fn((customType: string, data: unknown) => entries.push({ type: "custom", customType, data })) };
  register(pi as never);
  const ctx = { sessionManager: { getSessionId: () => "owner-test", getBranch: () => entries } };
  const fire = (event: string) => handlers.get(event)?.({}, ctx);
  cleanups.push(() => fire("session_shutdown"));
  return { pi, ctx, entries, fire };
}

describe("local cache ownership", () => {
  it("persists a non-message branch marker and restores it across reload", () => {
    const first = harness(); first.fire("session_start");
    const owner = localCacheOwner("owner-test");
    expect(owner?.session).toBe("owner-test");
    expect(first.entries[0].type).toBe("custom");
    first.fire("session_shutdown");
    expect(localCacheOwner("owner-test")).toBeUndefined();
    const next = harness();
    next.entries.push(...first.entries);
    next.fire("session_start");
    expect(localCacheOwner("owner-test")).toEqual(owner);
    expect(next.pi.appendEntry).not.toHaveBeenCalled();
    next.fire("session_tree");
    expect(localCacheOwner("owner-test")?.branch).not.toBe(owner?.branch);
  });

  it("does not let a metadata write failure stop session loading", () => {
    const h = harness();
    h.pi.appendEntry.mockImplementation(() => { throw new Error("disk full"); });
    expect(() => h.fire("session_start")).not.toThrow();
    expect(localCacheOwner("owner-test")).toBeUndefined();
  });

  it("supports disabling ownership without touching session metadata", () => {
    vi.stubEnv("OFFICINA_NO_CACHE_OWNERS", "1");
    const h = harness(); h.fire("session_start");
    expect(h.pi.appendEntry).not.toHaveBeenCalled();
    expect(localCacheOwner("owner-test")).toBeUndefined();
  });

  it("shares the branch identity between separate jiti extension instances", async () => {
    const one = await createJiti(import.meta.url, { moduleCache: false }).import<typeof import("./owner.ts")>("./owner.ts");
    const two = await createJiti(import.meta.url, { moduleCache: false }).import<typeof import("./owner.ts")>("./owner.ts");
    const h = harness(one.registerCacheOwners);
    h.fire("session_start");
    expect(two.localCacheOwner("owner-test")).toEqual(one.localCacheOwner("owner-test"));
    expect(two.localCacheOwner("owner-test")?.branch).toBeTruthy();
    h.fire("session_tree");
    expect(two.localCacheOwner("owner-test")?.branch).toBe(h.entries.at(-1).data.branch);
  });
});

const model = { api: "openai-completions" as const, provider: "llamacpp", id: "local", name: "Local",
  baseUrl: "http://memory.invalid/v1", reasoning: false, input: ["text"] as ["text"], contextWindow: 262144,
  maxTokens: 100, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } };

it.each(["llamacpp", "caduceus"])("scopes native owner metadata to the local provider: %s", async (provider) => {
  const h = harness(); h.fire("session_start");
  vi.stubEnv("OFFICINA_NO_PREFILL_PROGRESS", "1");
  let sent: any;
  const fetcher: typeof fetch = async (_url, init) => {
    sent = JSON.parse(init?.body as string);
    return new Response('data: {"id":"ok","choices":[{"index":0,"delta":{"content":"OK"},"finish_reason":null}]}\n\ndata: {"id":"ok","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n', { headers: { "content-type": "text/event-stream" } });
  };
  const context = normalizeContext({ messages: [{ role: "user", content: "hello", timestamp: 1 }] });
  const result = await streamLocalWithProgress({ ...model, provider }, context, { apiKey: "test", sessionId: "owner-test", fetch: fetcher }).result();
  expect(result.stopReason).toBe("stop");
  expect(sent.return_progress).toBeUndefined();
  expect(sent.cache_owner_session).toBe(provider === "llamacpp" ? "owner-test" : undefined);
  expect(sent.cache_owner_branch).toBe(provider === "llamacpp" ? localCacheOwner("owner-test")?.branch : undefined);
  expect(JSON.stringify(sent.messages)).not.toContain("owner-test");
});
