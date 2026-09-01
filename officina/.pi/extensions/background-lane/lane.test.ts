import { describe, expect, it } from "vitest";
import { PatchSink, buildReviewPrompt, laneConfig, renderCard, shouldLaunch } from "./lane.ts";

const cfg = { ...laneConfig({} as NodeJS.ProcessEnv), gate: "idle" as const };
const idle = (since: number | null, now = 10_000) =>
  shouldLaunch({ up: true, busy: 0, delta: { tps: 0 } }, 5000, since, now, cfg);

describe("laneConfig", () => {
  it("defaults on, kill-switches, parses slot", () => {
    expect(cfg.enabled).toBe(true);
    expect(laneConfig({ OFFICINA_NO_BACKGROUND: "1" } as NodeJS.ProcessEnv).enabled).toBe(false);
    expect(laneConfig({ OFFICINA_BG_SLOT: "0" } as NodeJS.ProcessEnv).slotId).toBe(0);
    expect(laneConfig({} as NodeJS.ProcessEnv).slotId).toBeUndefined();
  });
});

describe("shouldLaunch gate", () => {
  it("always gate launches even while the foreground decodes", () => {
    const acfg = { ...laneConfig({} as NodeJS.ProcessEnv), gate: "always" as const };
    expect(shouldLaunch({ up: true, busy: 1, delta: { tps: 12 } }, 5000, null, 0, acfg)).toBe(true);
    expect(shouldLaunch({ up: false, busy: 0, delta: { tps: 0 } }, 5000, null, 0, acfg)).toBe(false);
  });
  it("launches only when idle for idleMs with a big-enough patch", () => {
    expect(idle(6_000)).toBe(true); // idleMs=4000 default
    expect(idle(9_000)).toBe(false); // not idle long enough
    expect(idle(null)).toBe(false); // never saw idle
    expect(shouldLaunch({ up: false, busy: 0, delta: { tps: 0 } }, 5000, 6_000, 10_000, cfg)).toBe(false);
    expect(shouldLaunch({ up: true, busy: 1, delta: { tps: 0 } }, 5000, 6_000, 10_000, cfg)).toBe(false);
    expect(shouldLaunch({ up: true, busy: 0, delta: { tps: 12 } }, 5000, 6_000, 10_000, cfg)).toBe(false);
    expect(shouldLaunch({ up: true, busy: 0, delta: { tps: 0 } }, 100, 6_000, 10_000, cfg)).toBe(false);
  });
});

describe("buildReviewPrompt", () => {
  it("keeps the reviewer prefix byte-stable and appends the diff", () => {
    const a = buildReviewPrompt("diff A", cfg);
    const b = buildReviewPrompt("diff B", cfg);
    expect(a.startsWith(b.slice(0, b.indexOf("DIFF")))).toBe(true);
    expect(a.endsWith("diff A")).toBe(true);
  });
  it("truncates oversized patches", () => {
    const p = buildReviewPrompt("x".repeat(20_000), cfg);
    expect(p.length).toBeLessThan(400 + cfg.maxPatchChars + 40); // prefix + clipped diff
    expect(p).toContain("[truncated]");
  });
});

describe("renderCard", () => {
  it("returns null for CLEAN responses", () => {
    expect(renderCard("f", "CLEAN", cfg)).toBeNull();
    expect(renderCard("f", "clean.", cfg)).toBeNull();
    expect(renderCard("f", "", cfg)).toBeNull();
  });
  it("caps findings to budget", () => {
    const card = renderCard("f", "• " + "x".repeat(2000), cfg);
    expect(card).toBeTruthy();
    expect(card!.length).toBeLessThanOrEqual(cfg.cardBudgetChars + "[background review · f]\n".length + 1);
  });
});

describe("PatchSink", () => {
  it("coalesces files, drains once, respects min size", () => {
    const s = new PatchSink();
    s.add("a.ts", "patch-a");
    s.add("a.ts", "patch-a-again"); // dedup: one window per file
    s.add("b.py", "patch-b");
    expect(s.size).toBe(14); // "patch-a" (7) + "patch-b" (7)
    const out = s.drain(10, 50_000);
    expect(out).toContain("--- a.ts ---");
    expect(out).toContain("patch-b");
    expect(out).not.toContain("again");
    expect(s.drain(10, 50_000)).toBeNull(); // drained
  });
  it("returns null below minChars and truncates above maxChars", () => {
    const s = new PatchSink();
    s.add("a.ts", "tiny");
    expect(s.drain(200, 50_000)).toBeNull();
    const big = new PatchSink();
    big.add("a.ts", "y".repeat(100));
    expect(big.drain(10, 50)!.endsWith("[truncated]")).toBe(true);
  });
  it("ignores missing patches", () => {
    const s = new PatchSink();
    s.add("a.ts", undefined);
    expect(s.size).toBe(0);
  });
});
