// Integration: judgeWithDecision in each mode (2026-11-16).
// The point is the gate behaviour — a remote answer may only promote to healthy
// when it is confident, unambiguous, and (gemini mode) newly calibrated.
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { judgeWithDecision } from "./decision.ts";
import { writeDecisionConfig } from "./config.ts";
import type { WatchedCall } from "./judge.ts";

const call = (over: Partial<WatchedCall> = {}): WatchedCall => ({
  toolCallId: "t1",
  command: "cargo build --release",
  startedAt: Date.now() - 120_000,
  lastOutputRevision: 1,
  stalledSince: null,
  judgments: 0,
  lastVerdict: null,
  tail: "Compiling officina-tui v0.2.0",
  ...over,
});

let dir = "";
beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "officina-decision-mode-"));
  process.env.OFFICINA_WATCHDOG_DECISION_FILE = join(dir, "watchdog-decision.json");
});
afterEach(() => {
  delete process.env.OFFICINA_WATCHDOG_DECISION_FILE;
  delete process.env.GEMINI_API_KEY;
  delete process.env.OFFICINA_WATCHDOG_DECISION_MODE;
  vi.unstubAllGlobals();
  rmSync(dir, { recursive: true, force: true });
});

const writeGeminiConfig = () =>
  writeDecisionConfig(
    {
      version: 1,
      mode: "gemini",
      url: "",
      key: "test-gemini-key",
      floor: 0.6,
      margin: 0.15,
    },
    dir,
  );

describe("judgeWithDecision modes", () => {
  it("heuristic mode answers locally without any network call", async () => {
    const fetchSpy = vi.fn();
    vi.stubGlobal("fetch", fetchSpy);
    writeDecisionConfig(
      { version: 1, mode: "heuristic", url: "", key: "", floor: 0.9 },
      dir,
    );
    expect(
      await judgeWithDecision(call(), new AbortController().signal),
    ).toBeUndefined();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("gemini mode promotes a confident HUNG to a decision verdict", async () => {
    writeGeminiConfig();
    const fetchSpy = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            answers: {
              liveness: {
                choice: "HUNG",
                confidence: 0.94,
                probabilities: { ALIVE: 0.04, HUNG: 0.94, UNKNOWN: 0.02 },
              },
            },
          }),
          { status: 200 },
        ),
    );
    vi.stubGlobal("fetch", fetchSpy);
    const verdict = await judgeWithDecision(
      call(),
      new AbortController().signal,
    );
    expect(verdict?.source).toBe("decision");
    expect(verdict?.alive).toBe(false);
    expect(String(verdict?.reason)).toContain("HUNG");
    const firstCall = fetchSpy.mock.calls[0] as unknown as [string, RequestInit];
    const init = firstCall[1];
    const body = JSON.parse(String(init.body ?? "{}")) as { model?: string };
    expect(body.model).toBe("gemini-decision-1");
  });

  it("gemini mode abstains (unknown) when the answer is below the floor", async () => {
    writeGeminiConfig();
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              answers: {
                liveness: {
                  choice: "ALIVE",
                  confidence: 0.45,
                  probabilities: { ALIVE: 0.45, HUNG: 0.4, UNKNOWN: 0.15 },
                },
              },
            }),
            { status: 200 },
          ),
      ),
    );
    const verdict = await judgeWithDecision(
      call(),
      new AbortController().signal,
    );
    expect(verdict?.alive).toBeNull();
    expect(verdict?.source).toBe("decision");
    expect(String(verdict?.reason)).toContain("below floor");
  });

  it("gemini mode never claims health on a transport failure", async () => {
    writeGeminiConfig();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("{}", { status: 429 })),
    );
    const verdict = await judgeWithDecision(
      call(),
      new AbortController().signal,
    );
    expect(verdict?.alive).toBeNull();
    expect(String(verdict?.reason)).toContain("rate limited");
  });
});
