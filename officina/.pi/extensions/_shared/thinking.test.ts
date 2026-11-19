import { describe, expect, it } from "vitest";
import {
  DEFAULT_THINKING_BUDGET,
  MIN_USEFUL_THINKING_BUDGET,
  MIN_THINKING_BUDGET,
  answerRoom,
  applyThinkingInstruction,
  clampBudget,
  requestedBudget,
  thinkingEnabled,
} from "./thinking.ts";

const on = { OFFICINA_THINKING: "1" } as NodeJS.ProcessEnv;
const off = {} as NodeJS.ProcessEnv;

const withMarker = (marker: string, maxTokens = 4096) => ({
  messages: [
    { role: "system", content: `Judge carefully. ${marker}` },
    { role: "user", content: "go" },
  ],
  max_tokens: maxTokens,
});

describe("per-lane thinking (S4)", () => {
  it("sets enable_thinking AND a finite budget together, never one without the other", () => {
    // The whole invariant: a marker that enables thinking without a cap is the
    // original bug with better ergonomics (empty content at finish=length).
    const out = applyThinkingInstruction(withMarker("[[officina-think]]"), on);
    expect(out.chat_template_kwargs).toEqual({ enable_thinking: true });
    expect(typeof out.reasoning_budget_tokens).toBe("number");
    expect(out.reasoning_budget_tokens).toBe(DEFAULT_THINKING_BUDGET);
    expect(out.reasoning_budget_tokens).not.toBe(-1);
  });

  it("honours an explicit budget but clamps it to the room the answer needs", () => {
    const generous = applyThinkingInstruction(
      withMarker("[[officina-think 2048]]"),
      on,
    );
    expect(generous.reasoning_budget_tokens).toBe(2048);

    // max_tokens 1024 leaves 768 after the reserved answer room; asking for 2048
    // must not starve the answer it was asked to produce.
    const tight = applyThinkingInstruction(
      withMarker("[[officina-think 2048]]", 1024),
      on,
    );
    expect(tight.reasoning_budget_tokens).toBe(1024 - 256);
  });

  it("drops thinking entirely when there is no room for a useful budget", () => {
    // Measured (T2, 2026-11-19): a truncated thinker is the worst outcome — at
    // budget 64 the answer was EMPTY with finish_reason=stop, at 128 it was
    // confidently wrong. So a request that cannot afford MIN_USEFUL_THINKING_BUDGET
    // gets no thinker at all, and the marker is still consumed.
    const out = applyThinkingInstruction(
      withMarker("[[officina-think]]", 100),
      on,
    );
    expect(out.reasoning_budget_tokens).toBeUndefined();
    expect(out.chat_template_kwargs).toBeUndefined();
    const messages = out.messages as { content: string }[];
    expect(messages[0].content).not.toContain("officina-think");
  });

  it("defaults to the measured floor, not a guess", () => {
    // 256 is where hard tasks start being answered correctly; 1024 (the earlier
    // guess) spent 1.8x the thinking tokens for no accuracy gain.
    const out = applyThinkingInstruction(
      withMarker("[[officina-think]]", 2048),
      on,
    );
    expect(out.reasoning_budget_tokens).toBe(DEFAULT_THINKING_BUDGET);
    expect(out.reasoning_budget_tokens).toBe(MIN_USEFUL_THINKING_BUDGET);
  });

  it("is off unless the knob says otherwise (negative control)", () => {
    const payload = withMarker("[[officina-think]]");
    expect(thinkingEnabled(off)).toBe(false);
    expect(applyThinkingInstruction(payload, off)).toBe(payload);
  });

  it("leaves payloads without the marker untouched", () => {
    const plain = {
      messages: [{ role: "user", content: "no marker" }],
      max_tokens: 512,
    };
    expect(requestedBudget(plain)).toBeNull();
    expect(applyThinkingInstruction(plain, on)).toBe(plain);
  });

  it("defers to a caller that already decided enable_thinking (including 'off')", () => {
    const payload = {
      ...withMarker("[[officina-think 512]]"),
      chat_template_kwargs: { enable_thinking: false },
    };
    expect(applyThinkingInstruction(payload, on)).toBe(payload);
  });

  it("removes the marker from the prompt but keeps the rest of the text", () => {
    const out = applyThinkingInstruction(withMarker("[[officina-think]]"), on);
    const messages = out.messages as Array<{ role: string; content: string }>;
    expect(messages[0].content).toBe("Judge carefully.");
    expect(messages[0].content).not.toContain("officina-think");
  });

  it("derives the answer room from max_tokens and clamps both ways", () => {
    expect(answerRoom({ max_tokens: 4096 })).toBe(4096 - 256);
    expect(answerRoom({ max_completion_tokens: 1000 })).toBe(1000 - 256);
    expect(answerRoom({})).toBe(0);
    expect(clampBudget(10, 0)).toBe(MIN_THINKING_BUDGET);
    expect(clampBudget(100_000, 500)).toBe(500);
  });
});
