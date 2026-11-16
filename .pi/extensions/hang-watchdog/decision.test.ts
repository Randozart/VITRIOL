import { afterEach, describe, expect, it, vi } from "vitest";
import { decisionVerdict, judgeWithDecision, QUESTIONS } from "./decision.ts";
import type { WatchedCall } from "./judge.ts";

const complete = () => ({ usage: { truncated: false, state_tokens_dropped: 0, truncated_questions: [] }, answers: { liveness: { type: "choice", choice: "ALIVE", answer_confidence: 0.95, action: { act_probability: 0.99 }, abstention: "passed" } } });
const call: WatchedCall = { toolCallId: "call", command: "build", startedAt: 0, stalledSince: 0, judgments: 0, lastOutputRevision: 0, lastVerdict: null, tail: "compiling", generation: 2 };
afterEach(() => { vi.unstubAllEnvs(); vi.unstubAllGlobals(); });

describe("typed advisory decision endpoint", () => {
  it("accepts documented Laya choices, action objects and passed gates", () => {
    expect(decisionVerdict(complete(), 0.9).alive).toBe(true);
    const hung = complete(); hung.answers.liveness.choice = "HUNG";
    expect(decisionVerdict(hung, 0.9).alive).toBe(false);
  });
  it.each([
    ["malformed", (): null => null],
    ["lost state", () => { const d = complete(); d.usage.state_tokens_dropped = 1; return d; }],
    ["lost head", () => ({ ...complete(), usage: { ...complete().usage, truncated_questions: ["liveness"] } })],
    ["collapsed choices", () => ({ ...complete(), usage: { ...complete().usage, options: { liveness: { total: 3, distinct: 2 } } } })],
    ["abstained", () => { const d = complete(); d.answers.liveness.abstention = "abstained"; return d; }],
    ["unevaluated", () => { const d = complete(); d.answers.liveness.abstention = "unevaluated"; return d; }],
    ["UNKNOWN", () => { const d = complete(); d.answers.liveness.choice = "UNKNOWN"; return d; }],
    ["low answer score", () => { const d = complete(); d.answers.liveness.answer_confidence = 0.89; return d; }],
    ["low action score", () => { const d = complete(); d.answers.liveness.action.act_probability = 0.2; return d; }],
    ["nonfinite", () => { const d = complete(); d.answers.liveness.answer_confidence = NaN; return d; }],
    ["legacy entropy only", () => ({ ...complete(), answers: { liveness: { type: "choice", choice: "ALIVE", confidence: 1 } } })],
  ] as const)("keeps %s unknown", (_name, value) => { expect(decisionVerdict(value(), 0.9).alive).toBeNull(); });
  it("makes no request when unconfigured; sends compact evidence and caller cancellation when configured", async () => {
    vi.stubEnv("OFFICINA_WATCHDOG_DECISION_URL", "");
    const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify(complete()))); vi.stubGlobal("fetch", fetcher);
    const controller = new AbortController();
    expect(await judgeWithDecision(call, controller.signal)).toBeUndefined(); expect(fetcher).not.toHaveBeenCalled();
    vi.stubEnv("OFFICINA_WATCHDOG_DECISION_URL", "http://127.0.0.1:8000/v1/systemone");
    vi.stubEnv("OFFICINA_WATCHDOG_DECISION_KEY", "isolated-test");
    expect((await judgeWithDecision(call, controller.signal))?.alive).toBe(true);
    const [url, request] = fetcher.mock.calls[0];
    expect(String(url)).toBe("http://127.0.0.1:8000/v1/systemone");
    expect(request.signal).toBe(controller.signal);
    expect(request.headers.authorization).toBe("Bearer isolated-test");
    expect(JSON.parse(request.body)).toMatchObject({ questions: QUESTIONS, state: { command: "build", output_tail: "compiling", evidence_revision: 2 } });
  });
  it("returns UNKNOWN for service failure and invalid configuration", async () => {
    vi.stubEnv("OFFICINA_WATCHDOG_DECISION_URL", "http://127.0.0.1:8000/v1/systemone");
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("offline")));
    expect((await judgeWithDecision(call, new AbortController().signal))?.alive).toBeNull();
    vi.stubEnv("OFFICINA_WATCHDOG_DECISION_MIN_SCORE", "NaN");
    expect((await judgeWithDecision(call, new AbortController().signal))?.reason).toContain("invalid");
  });
});
