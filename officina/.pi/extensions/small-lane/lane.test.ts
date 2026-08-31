import { describe, expect, it } from "vitest";
import { buildSummaryText, laneCompactionOutcome, resolveLaneConfig } from "./lane.ts";

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
