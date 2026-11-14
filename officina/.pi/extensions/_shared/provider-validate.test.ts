import { describe, expect, it } from "vitest";
import {
  looksLikeCompletion,
  renderViolations,
  validateChatCompletion,
  validateStreamChunk,
} from "./provider-validate.ts";

describe("validateChatCompletion", () => {
  const good = {
    id: "x",
    model: "m",
    choices: [
      {
        index: 0,
        message: { role: "assistant", content: "hi" },
        finish_reason: "stop",
      },
    ],
    usage: { prompt_tokens: 10, completion_tokens: 2, total_tokens: 12 },
  };

  it("accepts a well-formed completion", () => {
    expect(validateChatCompletion(good)).toEqual({ ok: true, violations: [] });
  });

  it("rejects structurally broken payloads with typed codes", () => {
    expect(validateChatCompletion(null).violations[0]?.code).toBe("not-object");
    const noChoices = validateChatCompletion({ id: "x" });
    expect(noChoices.ok).toBe(false);
    expect(noChoices.violations.some((v) => v.code === "choices")).toBe(true);
    const badMessage = validateChatCompletion({
      choices: [{ index: 0, finish_reason: "stop" }],
    });
    expect(badMessage.violations.some((v) => v.code === "message")).toBe(true);
    const badFinish = validateChatCompletion({
      choices: [{ message: { role: "assistant" }, finish_reason: "exploded" }],
    });
    expect(badFinish.violations.some((v) => v.code === "finish-reason")).toBe(true);
    const badUsage = validateChatCompletion({
      choices: [{ message: { role: "assistant" }, finish_reason: "stop" }],
      usage: { prompt_tokens: "ten" },
    });
    expect(badUsage.violations.some((v) => v.code === "usage-number")).toBe(true);
  });
});

describe("validateStreamChunk", () => {
  it("accepts deltas and rejects malformed chunks", () => {
    expect(validateStreamChunk({ choices: [{ index: 0, delta: { content: "a" } }] }).ok).toBe(true);
    expect(validateStreamChunk({ choices: "nope" }).violations[0]?.code).toBe("choices");
    expect(validateStreamChunk({ choices: [{ delta: 7 }] }).violations[0]?.code).toBe("delta");
  });

  it("renders bounded violation summaries", () => {
    const text = renderViolations(
      [{ code: "a", detail: "one" }, { code: "b", detail: "two" }],
      1,
    );
    expect(text).toBe("a: one");
  });

  it("recognises completion-shaped payloads", () => {
    expect(looksLikeCompletion({ choices: [] })).toBe(true);
    expect(looksLikeCompletion({ error: "x" })).toBe(false);
  });
});
