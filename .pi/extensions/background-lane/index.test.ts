import { afterEach, expect, it, vi } from "vitest";
import register from "./index.ts";

const engine = vi.hoisted(() => ({ up: true, busy: 0, delta: { tps: 0 }, slots: [] as unknown[] | undefined }));
vi.mock("../_shared/engine.ts", () => ({ startEnginePolling: () => {}, getEngineSnapshot: () => engine }));
vi.mock("../_shared/events.ts", () => ({ emitHarnessEvent: () => {}, harnessEvent: () => ({}) }));
afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); vi.unstubAllEnvs(); vi.unstubAllGlobals(); });

it.each([undefined, [], [{ id: 0 }]])("retains patches without submitting HTTP until multi-slot capacity is known %#", async (slots) => {
  vi.useFakeTimers();
  vi.stubEnv("OFFICINA_NO_BACKGROUND", "0");
  vi.stubEnv("OFFICINA_BG_GATE", "always");
  vi.stubEnv("OFFICINA_BG_IDLE_MS", "2000");
  engine.slots = slots;
  const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify({ content: "CLEAN" })));
  vi.stubGlobal("fetch", fetcher);
  const handlers = new Map<string, (event: any, ctx: any) => unknown>();
  register({ on: (name: string, handler: (event: any, ctx: any) => unknown) => { handlers.set(name, handler); } } as never);
  handlers.get("session_start")!({}, { hasUI: true, cwd: "/unused", sessionManager: { getSessionId: () => "test" } });
  const patch = "meaningful patch\n".repeat(30);
  handlers.get("tool_result")!({ toolName: "edit", input: { path: "example.ts" }, details: { patch } }, {});
  await vi.advanceTimersByTimeAsync(5000);
  expect(fetcher).not.toHaveBeenCalled();
  engine.slots = [{ id: 0 }, { id: 1 }];
  await vi.advanceTimersByTimeAsync(1000);
  expect(fetcher).toHaveBeenCalledTimes(1);
  const sent = JSON.parse(fetcher.mock.calls[0][1].body);
  expect(sent.prompt).toContain(patch.trim());
  // The patch was drained once after admission, rather than discarded while suppressed.
  await vi.advanceTimersByTimeAsync(5000);
  expect(fetcher).toHaveBeenCalledTimes(1);
});
