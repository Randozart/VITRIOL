import { describe, expect, it, vi } from "vitest";
import type { ExtensionAPI, SessionBoundaryDraft } from "@earendil-works/pi-coding-agent";
import { SessionManager } from "../../../node_modules/@earendil-works/pi-coding-agent/dist/core/session-manager.js";
import { ExtensionRunner } from "../../../node_modules/@earendil-works/pi-coding-agent/dist/core/extensions/runner.js";
import { registerContextState } from "./context-state.ts";
import { shouldDeferToolClear } from "./cache-mode.ts";
import { clearConfig, planClear } from "../tool-result-clearer/index.ts";
import ledgerExtension from "../session-ledger/index.ts";

vi.mock("./events.ts", () => ({ emitHarnessEvent: vi.fn(), harnessEvent: vi.fn() }));

type Handler = (event: never, ctx: never) => unknown;
type Message = Parameters<SessionManager["appendMessage"]>[0];

/** Exercise installed pi request cloning and boundary dispatch with an in-memory session. */
function harness(blocks: Record<string, string>, setup?: (pi: ExtensionAPI) => void) {
  const manager = SessionManager.inMemory("/tmp");
  const handlers = new Map<string, Handler[]>();
  const pi = {
    on: (name: string, handler: Handler) => {
      handlers.set(name, [...(handlers.get(name) ?? []), handler]);
    },
  } as unknown as ExtensionAPI;
  for (const key of Object.keys(blocks)) registerContextState(pi, key, () => blocks[key]);
  setup?.(pi);
  const runner = {
    extensions: [{ path: "context-state-test", handlers }],
    createContext: () => ({ sessionManager: manager }),
    emitError: (error: { message: string }) => { throw new Error(error.message); },
  } as unknown as ExtensionRunner;
  const messages = () => manager.buildSessionProjection().messages;
  const persist = (entries: SessionBoundaryDraft[]) => {
    for (const entry of entries) {
      if (entry.type !== "custom_message") throw new Error("unexpected draft");
      manager.appendCustomMessageEntry(entry.customType, entry.content, entry.display, entry.details);
    }
  };
  const request = () => ExtensionRunner.prototype.emitContext.call(runner, messages());
  const start = async () => {
    const result = await ExtensionRunner.prototype.emitBeforeAgentStart.call(
      runner, { messageType: "user", prompt: "work" }, { cwd: "/tmp" },
    );
    manager.appendMessage({ role: "user", content: "work", timestamp: 1 } as Message);
    persist(result.messages.map((message) => ({ type: "custom_message", ...message })));
  };
  const finish = async () => {
    const result = await ExtensionRunner.prototype.emitBoundary.call(
      runner,
      { type: "turn_end", turnIndex: 0, message: {} as Message, toolResults: [], messageEntryId: "test", toolResultEntryIds: [], outcome: "completed" },
      () => ({ contextEntries: [], contextMessages: messages(), llmMessages: [], pendingMessages: [], canContinue: true }),
    );
    persist(result.entries);
    return result;
  };
  const toolStep = (id: string) => {
    manager.appendMessage({
      role: "assistant", content: [{ type: "toolCall", id, name: "read", arguments: { path: "a" } }],
      timestamp: 2, api: "openai-completions", provider: "llamacpp", model: "test", stopReason: "toolUse",
      usage: { input: 1, output: 1, cacheRead: 0, cacheWrite: 0, totalTokens: 2,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
    });
    manager.appendMessage({ role: "toolResult", toolCallId: id, toolName: "read", content: [{ type: "text", text: "output" }], isError: false, timestamp: 3 } as Message);
  };
  return { manager, request, start, finish, toolStep, messages };
}

describe("durable context under installed pi", () => {
  it("preserves the full previous request with the real session ledger enabled", async () => {
    const h = harness({ "lc-scratchpad": "facts", "lc-tasks": "task" }, ledgerExtension);
    await h.start();
    let previous = await h.request();
    for (const id of ["one", "two", "three"]) {
      h.toolStep(id);
      const boundary = await h.finish();
      expect(boundary.continue).toBe(false);
      const next = await h.request();
      expect(next.slice(0, previous.length)).toEqual(previous);
      previous = next;
    }
    expect(previous.filter((m) => m.role === "custom" && m.customType === "lc-ledger")).toHaveLength(4);
  });
  it("retains the previous request over repeated tool continuations and user turns", async () => {
    const h = harness({ "lc-scratchpad": "facts", "lc-tasks": "task", "agents-md": "rules" });
    await h.start();
    let previous = await h.request();
    for (const id of ["one", "two", "three"]) {
      h.toolStep(id);
      const boundary = await h.finish();
      expect(boundary.entries).toEqual([]);
      expect(boundary.continue).toBe(false);
      const next = await h.request();
      expect(next.slice(0, previous.length)).toEqual(previous);
      expect(next.filter((m) => m.role === "custom")).toHaveLength(3);
      previous = next;
    }
    await h.start();
    expect((await h.request()).slice(0, previous.length)).toEqual(previous);
  });

  it("appends all changed snapshots after tools without requesting inference", async () => {
    const blocks = { scratch: "old", tasks: "old" };
    const h = harness(blocks);
    await h.start();
    const previous = await h.request();
    h.toolStep("update");
    blocks.scratch = "new facts";
    blocks.tasks = "new tasks";
    const result = await h.finish();
    expect(result.continue).toBe(false);
    expect(result.entries).toHaveLength(2);
    const next = await h.request();
    expect(next.slice(0, previous.length)).toEqual(previous);
    expect(next.slice(-3).map((m) => m.role)).toEqual(["toolResult", "custom", "custom"]);
    expect(next.slice(-2).map((m) => "content" in m ? m.content : undefined)).toEqual(["new facts", "new tasks"]);
  });

  it("recovers discarded state until the next safe persistence boundary", async () => {
    const h = harness({ scratch: "facts" });
    await h.start();
    h.manager.appendCompaction("compacted summary", null, 100);
    const recovery = await h.request();
    expect(recovery.at(-1)).toMatchObject({ content: "facts" });
    expect(h.messages().some((m) => m.role === "custom")).toBe(false);
    h.toolStep("recover");
    expect((await h.finish()).entries).toHaveLength(1);
    const stable = await h.request();
    h.toolStep("next");
    await h.finish();
    expect((await h.request()).slice(0, stable.length)).toEqual(stable);
  });

  it("records an explicit reset when previously nonempty state is cleared", async () => {
    const blocks = { scratch: "obsolete facts" };
    const h = harness(blocks);
    await h.start();
    blocks.scratch = "";
    const result = await h.finish();
    expect(result.entries).toHaveLength(1);
    expect((await h.request()).at(-1)).toMatchObject({ content: expect.stringContaining("state cleared") });
    expect((await h.finish()).entries).toEqual([]);
  });

  it("detects the old moving-tail failure when persistence hooks are absent", async () => {
    const h = harness({ scratch: "facts" });
    h.manager.appendMessage({ role: "user", content: "work", timestamp: 1 } as Message);
    const previous = await h.request();
    h.toolStep("old-behavior");
    const next = await h.request();
    expect(next.slice(0, previous.length)).not.toEqual(previous);
    expect(h.messages().some((m) => m.role === "custom")).toBe(false);
  });

  it("does not leak snapshots into a different session or emit empty state", async () => {
    const first = harness({ scratch: "secret facts" });
    await first.start();
    const second = harness({ scratch: "" });
    await second.start();
    expect((await second.request()).some((m) => m.role === "custom")).toBe(false);
    expect((await second.finish()).entries).toEqual([]);
  });

  it("shows why earlier clearing trades prefix reuse for a shorter prompt", () => {
    const h = harness({});
    h.toolStep("old");
    h.toolStep("recent");
    const original = h.messages();
    const plan = planClear(original, { ...clearConfig({}), keep: 1 });
    expect(plan.cleared).toBe(1);
    expect(plan.messages[0]).toEqual(original[0]);
    expect(plan.messages[1]).not.toEqual(original[1]);
    expect(original[1]).toMatchObject({ content: [{ type: "text", text: "output" }] });
    expect(shouldDeferToolClear(80, {})).toBe(true);
    expect(shouldDeferToolClear(80, { OFFICINA_CACHE_CLEAR_PCT: "75" })).toBe(false);
  });
});
