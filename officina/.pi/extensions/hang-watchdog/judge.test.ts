// hang-watchdog rules (2026-11-12): observe → notice → judge. NEVER kills.
import { describe, expect, it } from "vitest";
import {
  buildJudgePrompt,
  MAX_JUDGMENTS_PER_CALL,
  parseVerdict,
  pickJudgeModel,
  short,
  tickCall,
  type WatchedCall,
} from "./judge.ts";

const T0 = 1_000_000_000_000;
const call = (over: Partial<WatchedCall> = {}): WatchedCall => ({
  toolCallId: "t1",
  command: "brievc build c4probe.bv --verify-bodies",
  startedAt: T0,
  lastOutputChars: 0,
  stalledSince: null,
  judgments: 0,
  lastVerdict: null,
  ...over,
});

describe("tickCall", () => {
  it("stays silent before the soft threshold", () => {
    const c = call();
    expect(tickCall(c, 0, "", { now: T0 + 30_000, softS: 60, judgeS: 180 }).kind).toBe("none");
  });

  it("notices once when output stalls past soft", () => {
    const c = call();
    const a = tickCall(c, 0, "", { now: T0 + 61_000, softS: 60, judgeS: 180 });
    expect(a.kind).toBe("notice");
    expect((a as { message: string }).message).toContain("no output for 61s");
    // second tick: no repeat
    expect(tickCall(c, 0, "", { now: T0 + 90_000, softS: 60, judgeS: 180 }).kind).toBe("none");
  });

  it("growing output resets the stall and suppresses escalation", () => {
    const c = call({ stalledSince: T0 + 61_000, lastVerdict: "noticed" });
    const a = tickCall(c, 500, "more output", { now: T0 + 120_000, softS: 60, judgeS: 180 });
    expect(a.kind).toBe("alive");
    expect(c.stalledSince).toBeNull();
    expect(c.lastOutputChars).toBe(500);
  });

  it("escalates to a judgment after judgeS of stall", () => {
    const c = call({ lastVerdict: "noticed", stalledSince: T0, lastOutputChars: 10 });
    const a = tickCall(c, 10, "some tail", { now: T0 + 181_000, softS: 60, judgeS: 180 });
    expect(a.kind).toBe("judge");
    expect((a as { tail: string }).tail).toBe("some tail");
    expect(c.judgments).toBe(1);
  });

  it("re-judges only after the re-judge cadence and caps per call", () => {
    const c = call({
      lastVerdict: "HUNG",
      stalledSince: T0,
      lastOutputChars: 10,
      judgments: 1,
      judgedAt: T0 + 181_000,
    });
    expect(
      tickCall(c, 10, "", { now: T0 + 240_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("none"); // too soon after the last judgment
    const a = tickCall(c, 10, "", { now: T0 + 500_000, softS: 60, judgeS: 180 });
    expect(a.kind).toBe("judge");
    expect(c.judgments).toBe(2);
    c.judgments = MAX_JUDGMENTS_PER_CALL;
    expect(tickCall(c, 10, "", { now: T0 + 900_000, softS: 60, judgeS: 180 }).kind).toBe("none");
  });
});

describe("parseVerdict", () => {
  it("reads the first VERDICT line and fails open", () => {
    expect(parseVerdict("VERDICT: HUNG\nIt waits on stdin.")).toEqual({
      alive: false,
      reason: "It waits on stdin.",
    });
    expect(parseVerdict("noise\nVERDICT: ALIVE\ncompiling links").alive).toBe(true);
    expect(parseVerdict("garbage").alive).toBe(true);
    expect(parseVerdict("garbage").reason).toContain("unparseable");
  });
});

describe("buildJudgePrompt", () => {
  it("carries command, timings, tail and the one-line contract", () => {
    const p = buildJudgePrompt("brievc build", 300, 200, "partial output\n");
    expect(p).toContain("brievc build");
    expect(p).toContain("300s");
    expect(p).toContain("200s");
    expect(p).toContain("VERDICT: ALIVE or VERDICT: HUNG");
    const empty = buildJudgePrompt("x", 1, 1, "   ");
    expect(empty).toContain("(no output at all");
    expect(empty).not.toContain("partial output");
  });
});

describe("pickJudgeModel + short", () => {
  it("picks the best free lane or null", () => {
    expect(pickJudgeModel(["a/b", "c/d"])).toBe("a/b");
    expect(pickJudgeModel([])).toBeNull();
    expect(short("a  b\n c")).toBe("a b c");
  });
});
