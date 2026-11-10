import { describe, expect, it } from "vitest";
import { computeCacheReport, parseSessionJsonl } from "./cache-report-lib.mjs";

function msg(usage, extra = {}) {
  return {
    type: "message",
    message: {
      role: "assistant",
      provider: "caduceus",
      model: "ddgw/gpt-5.4-mini",
      timestamp: 1000,
      usage,
      ...extra,
    },
  };
}

describe("parseSessionJsonl", () => {
  it("ignores blank/truncated lines", () => {
    expect(parseSessionJsonl('{"type":"x"}\nnot-json\n')).toHaveLength(1);
  });
});

describe("computeCacheReport", () => {
  it("counts cache reads/writes and hit rate", () => {
    const report = computeCacheReport([
      msg({ input: 100, output: 10, cacheRead: 900, cacheWrite: 0, cost: { input: 1, output: 2, cacheRead: 0.1, cacheWrite: 0 } }),
    ]);
    expect(report.turns).toBe(1);
    expect(report.cacheRead).toBe(900);
    expect(report.promptTokens).toBe(1000);
    expect(report.hitRate).toBeCloseTo(0.9);
  });

  it("detects a significant miss and model switch", () => {
    const report = computeCacheReport([
      msg({ input: 100, output: 1, cacheRead: 1900, cacheWrite: 0, cost: { input: 1, output: 0, cacheRead: 0.1, cacheWrite: 0 } }),
      msg(
        { input: 2000, output: 1, cacheRead: 0, cacheWrite: 0, cost: { input: 1, output: 0, cacheRead: 0, cacheWrite: 0 } },
        { model: "other", timestamp: 2000 },
      ),
    ]);
    expect(report.missCount).toBe(1);
    expect(report.missedTokens).toBe(2000);
    expect(report.modelSwitches).toBe(1);
  });

  it("resets at compaction", () => {
    const report = computeCacheReport([
      msg({ input: 100, output: 1, cacheRead: 900, cacheWrite: 0 }),
      { type: "compaction" },
      msg({ input: 1000, output: 1, cacheRead: 0, cacheWrite: 0 }, { timestamp: 2000 }),
    ]);
    expect(report.missCount).toBe(0);
  });
});
