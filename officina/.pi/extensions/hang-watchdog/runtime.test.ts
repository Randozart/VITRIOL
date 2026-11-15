import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import install, { watchdogPath } from "./index.ts";

const catalog = vi.hoisted(() => ({ candidates: [{ id: "test/free" }] }));
vi.mock("../caduceus/catalog.ts", () => ({ extractCatalogModels: (value: unknown) => value, freeToolCandidates: () => catalog.candidates }));
let home: string;
let handlers: Map<string, (...args: any[]) => any>;
let notify: ReturnType<typeof vi.fn>;
let fetcher: ReturnType<typeof vi.fn>;
const fire = (event: string, payload: unknown = {}, ctx?: unknown) => handlers.get(event)?.(payload, ctx);
const session = (id = "session-a") => fire("session_start", {}, { hasUI: false, ui: { notify }, sessionManager: { getSessionId: () => id } });
const start = () => fire("tool_execution_start", { toolName: "bash", toolCallId: "tool-1", args: { command: "build --quiet" } });
const facts = (id = "session-a") => JSON.parse(readFileSync(watchdogPath(id), "utf8"));
const response = (content: string) => new Response(JSON.stringify({ choices: [{ message: { content } }] }));

beforeEach(() => {
  vi.useFakeTimers(); vi.setSystemTime(1_800_000_000_000);
  home = mkdtempSync(join(tmpdir(), "officina-watchdog-"));
  vi.stubEnv("OFFICINA_WATCHDOG_DECISION_URL", "");
  vi.stubEnv("HOME", home); vi.stubEnv("OFFICINA_CADUCEUS_API_KEY", "test");
  vi.stubEnv("OFFICINA_NO_HANG_WATCHDOG", "0");
  handlers = new Map(); notify = vi.fn();
  fetcher = vi.fn().mockResolvedValue(response("VERDICT: HUNG\nwaiting for stdin"));
  vi.stubGlobal("fetch", fetcher); catalog.candidates = [{ id: "test/free" }];
  install({ on: (event: string, handler: (...args: any[]) => any) => handlers.set(event, handler) } as unknown as ExtensionAPI);
  session();
});
afterEach(async () => {
  fire("session_shutdown"); await Promise.resolve();
  vi.useRealTimers(); vi.unstubAllGlobals(); vi.unstubAllEnvs(); rmSync(home, { recursive: true, force: true });
});

// The real catalog read remains bounded/local; create the isolated expected path.
function availableCatalog(): void {
  mkdirSync(join(home, ".vitriol", "officina"), { recursive: true });
  writeFileSync(join(home, ".vitriol", "officina", "caduceus-models.json"), "{}");
}

describe("watchdog runtime", () => {
  it("uses an explicit typed endpoint without cloud fallback when it abstains", async () => {
    availableCatalog();
    vi.stubEnv("OFFICINA_WATCHDOG_DECISION_URL", "http://127.0.0.1:8000/v1/systemone");
    fetcher.mockResolvedValue(new Response(JSON.stringify({ answers: {} })));
    start(); await vi.advanceTimersByTimeAsync(180_000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(String(fetcher.mock.calls[0][0])).toContain("/v1/systemone");
    expect(facts().verdict).toBe("UNKNOWN");
  });
  it("surfaces notices in RPC/headless contexts and never turns malformed replies into health", async () => {
    availableCatalog(); start(); fetcher.mockResolvedValue(response("not a verdict"));
    await vi.advanceTimersByTimeAsync(180_000);
    expect(notify).toHaveBeenCalled();
    expect(facts().verdict).toBe("UNKNOWN");
    expect(facts().kind).toBe("unavailable");
    expect(facts().next_check_at).toBeGreaterThan(Date.now());
    expect(facts().session_id).toBe("session-a");
  });

  it("tracks object-shaped, constant-length output and suppresses duplicate callbacks", async () => {
    availableCatalog(); start();
    for (let i = 0; i < 10; i++) {
      await vi.advanceTimersByTimeAsync(30_000);
      fire("tool_execution_update", { toolCallId: "tool-1", partialResult: { content: [{ type: "text", text: String(i).repeat(2000) }] } });
    }
    expect(fetcher).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(120_000);
    fire("tool_execution_update", { toolCallId: "tool-1", partialResult: { content: [{ type: "text", text: "9".repeat(2000) }] } });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("bounds a judge that ignores abort and continues deterministic observation after the budget", async () => {
    availableCatalog(); start(); fetcher.mockImplementation(() => new Promise(() => {}));
    await vi.advanceTimersByTimeAsync(180_000);
    expect(facts().verdict).toBe("JUDGING");
    await vi.advanceTimersByTimeAsync(31_000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(facts().verdict).toBe("UNKNOWN");
    await vi.advanceTimersByTimeAsync(900_000);
    expect(fetcher).toHaveBeenCalledTimes(3);
    expect(facts().phase).toBe("monitor");
    expect(facts().next_check_at).toBeGreaterThan(Date.now());
  });

  it("invalidates delayed verdicts on progress and completion", async () => {
    availableCatalog(); start();
    let resolve!: (value: Response) => void;
    fetcher.mockReturnValue(new Promise<Response>((done) => { resolve = done; }));
    await vi.advanceTimersByTimeAsync(180_000);
    fire("tool_execution_update", { toolCallId: "tool-1", partialResult: { content: [{ type: "text", text: "compiled" }] } });
    await vi.advanceTimersByTimeAsync(0);
    resolve(response("VERDICT: HUNG\nold evidence"));
    await vi.advanceTimersByTimeAsync(0);
    expect(facts().verdict).toBe("ALIVE");
    expect(facts().reason).toContain("new output");
    fire("tool_execution_end", { toolCallId: "tool-1" });
    expect(facts().kind).toBe("ended");
    expect(facts().next_check_at).toBe(0);
  });

  it("does not publish an old session's reply into a new session and tears down timers", async () => {
    availableCatalog(); start();
    let resolve!: (value: Response) => void;
    fetcher.mockReturnValue(new Promise<Response>((done) => { resolve = done; }));
    await vi.advanceTimersByTimeAsync(180_000);
    session("session-b"); resolve(response("VERDICT: HUNG old session"));
    await vi.advanceTimersByTimeAsync(0);
    expect(existsSync(watchdogPath("session-b"))).toBe(false);
    expect(facts("session-a").verdict).toBe("JUDGING");
    fire("session_shutdown");
    expect(vi.getTimerCount()).toBe(0);
    session("session-c"); start();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(facts("session-c").kind).toBe("notice");
  });

  it("keeps watching when no judge lane exists", async () => {
    catalog.candidates = []; start();
    await vi.advanceTimersByTimeAsync(180_000);
    expect(facts().verdict).toBe("UNKNOWN");
    expect(fetcher).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(900_000);
    expect(facts().phase).toBe("monitor");
    expect(facts().verdict).toBe("UNKNOWN");
  });
});
