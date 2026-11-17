import { chooseSummarizer, estimateMessagesTokens, shrinkMessages } from "./lane.ts";
import { describe, expect, it } from "vitest";
import { buildSummaryText, fallbackDetail, laneCompactionOutcome, resolveLaneConfig } from "./lane.ts";

describe("resolveLaneConfig", () => {
  it("is hard off with the kill switch (Rule 15)", () => {
    const cfg = resolveLaneConfig({ LITTLE_CODER_NO_SMALL_LANE: "1" });
    expect(cfg.enabled).toBe(false);
  });

  it("defaults to the VITRIOL mellum2 lane", () => {
    const cfg = resolveLaneConfig({});
    expect(cfg.enabled).toBe(true);
    expect(cfg.baseUrl).toBe("http://127.0.0.1:8287/v1");
    expect(cfg.modelId).toBe("mellum2-instruct");
    expect(cfg.contextWindow).toBe(131072);
  });

  it("honors env overrides", () => {
    const cfg = resolveLaneConfig({
      LITTLE_CODER_SMALL_LANE_URL: "http://127.0.0.1:9999/v1",
      LITTLE_CODER_SMALL_LANE_MODEL: "other-model",
      LITTLE_CODER_SMALL_LANE_CTX: "32768",
    });
    expect(cfg.baseUrl).toBe("http://127.0.0.1:9999/v1");
    expect(cfg.modelId).toBe("other-model");
    expect(cfg.contextWindow).toBe(32768);
  });

  it("keeps defaults on garbage values", () => {
    const cfg = resolveLaneConfig({ LITTLE_CODER_SMALL_LANE_CTX: "not-a-number" });
    expect(cfg.contextWindow).toBe(131072);
  });
});

describe("buildSummaryText", () => {
  it("embeds the conversation and no previous-context slot", () => {
    const text = buildSummaryText("CONV BODY");
    expect(text).toContain("CONV BODY");
    expect(text).not.toContain("{{CONVERSATION}}");
    expect(text).not.toContain("Previous session summary");
  });

  it("includes the previous summary when present", () => {
    const text = buildSummaryText("CONV BODY", "OLD SUMMARY");
    expect(text).toContain("Previous session summary for context:\nOLD SUMMARY");
    expect(text).toContain("CONV BODY");
  });
});

describe("laneCompactionOutcome", () => {
  it("uses a real summary", () => {
    expect(laneCompactionOutcome("## Summary\nwork happened")).toBe("use");
  });

  it("falls back on empty output (a blank summary must not eat history)", () => {
    expect(laneCompactionOutcome("")).toBe("fallback");
    expect(laneCompactionOutcome("   \n  ")).toBe("fallback");
    expect(laneCompactionOutcome(null)).toBe("fallback");
    expect(laneCompactionOutcome(undefined)).toBe("fallback");
  });
});

describe("estimateMessagesTokens", () => {
  it("counts text, thinking and tool arguments at chars/4", () => {
    const msgs = [
      { content: "abcd".repeat(25) }, // 100 chars
      { content: [{ type: "thinking", thinking: "x".repeat(200) }] },
      { content: [{ type: "toolCall", input: { a: "b".repeat(100) } }] },
    ];
    const est = estimateMessagesTokens(msgs);
    expect(est).toBeGreaterThanOrEqual(90);
    expect(est).toBeLessThanOrEqual(130);
  });
});

describe("chooseSummarizer", () => {
  const models = [
    { provider: "caduceus", id: "freeinference/qwen3.6-35b", contextWindow: 128000 },
    { provider: "caduceus", id: "agentrouter/gpt-5.6-sol", contextWindow: 1050000 },
    { provider: "llamacpp", id: "Lapis Occultus", contextWindow: 262144 },
  ];

  it("picks the largest window that fits estimate + reserve", () => {
    expect(chooseSummarizer(models, 183_106)?.id).toBe("agentrouter/gpt-5.6-sol");
    expect(chooseSummarizer(models, 20_000)?.id).toBe("agentrouter/gpt-5.6-sol");
  });

  it("returns undefined when nothing fits", () => {
    expect(chooseSummarizer(models, 2_000_000)).toBeUndefined();
  });

  it("prefers cloud on equal windows", () => {
    const tie = [
      { provider: "llamacpp", id: "Lapis Occultus", contextWindow: 262144 },
      { provider: "caduceus", id: "caduceus/other", contextWindow: 262144 },
    ];
    expect(chooseSummarizer(tie, 50_000)?.provider).toBe("caduceus");
  });
});

describe("shrinkMessages", () => {
  it("keeps the newest messages within the budget and digests the rest", () => {
    const messages = Array.from({ length: 10 }, (_, i) => ({
      role: i % 2 ? "assistant" : "user",
      content: `message ${i} ${"x".repeat(100)}`,
    }));
    const out = shrinkMessages(messages, 450);
    expect(out.omitted).toBeGreaterThan(0);
    expect(out.messages.length).toBeLessThan(messages.length);
    expect(out.digest).toContain("omitted");
    expect(out.digest).toContain("message 0");
    // The kept window is the newest tail.
    const last = out.messages[out.messages.length - 1] as { content: string };
    expect(last.content).toContain("message 9");
  });

  it("is a no-op when everything fits", () => {
    const messages = [{ role: "user", content: "hi" }];
    const out = shrinkMessages(messages, 10_000);
    expect(out.omitted).toBe(0);
    expect(out.messages).toEqual(messages);
  });
});

// Regression (2026-11-17): a compaction fallback was invisible — the only trace
// was a generic notice, so a 300 s client timeout on a deep local prefill read
// as "compaction is broken". The fallback line must name the chain, the error,
// and the fix when the error is a timeout.
describe("fallbackDetail", () => {
  it("names every summarizer that was tried and the last error", () => {
    const detail = fallbackDetail(
      [{ provider: "caduceus", id: "agnes-3.0-flash" }, { provider: "llamacpp", id: "Lapis Occultus" }],
      "429 rate limited",
    );
    expect(detail).toContain("caduceus/agnes-3.0-flash → llamacpp/Lapis Occultus");
    expect(detail).toContain("429 rate limited");
    expect(detail).not.toContain("httpIdleTimeoutMs"); // not a timeout: no red herring
  });

  it("points at the timeout fix only when the client timed out", () => {
    const detail = fallbackDetail([{ provider: "llamacpp", id: "Lapis Occultus" }], "Request timed out.");
    expect(detail).toContain("Request timed out.");
    expect(detail).toContain("httpIdleTimeoutMs");
  });

  it("survives an empty chain and an empty error", () => {
    expect(fallbackDetail([], "")).toBe("tried none: no error reported");
  });
});
