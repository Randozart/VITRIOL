// hang-watchdog rules (2026-11-12): observe → notice → judge. NEVER kills.
import { describe, expect, it } from "vitest";
import {
  observeToolOutput,
  toolOutputText,
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
  lastOutputRevision: 0,
  stalledSince: null,
  judgments: 0,
  lastVerdict: null,
  ...over,
});

describe("tickCall", () => {
  it("stays silent before the soft threshold", () => {
    const c = call();
    expect(
      tickCall(c, 0, "", { now: T0 + 30_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("none");
  });

  it("notices once when output stalls past soft", () => {
    const c = call();
    const a = tickCall(c, 0, "", { now: T0 + 61_000, softS: 60, judgeS: 180 });
    expect(a.kind).toBe("notice");
    expect((a as { message: string }).message).toContain("no output for 61s");
    // second tick: no repeat
    expect(
      tickCall(c, 0, "", { now: T0 + 90_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("none");
  });

  it("growing output resets the stall and suppresses escalation", () => {
    const c = call({ stalledSince: T0 + 61_000, lastVerdict: "noticed" });
    const a = tickCall(c, 500, "more output", {
      now: T0 + 120_000,
      softS: 60,
      judgeS: 180,
    });
    expect(a.kind).toBe("alive");
    expect(c.stalledSince).toBeNull();
    expect(c.lastOutputRevision).toBe(500);
  });

  it("escalates to a judgment after judgeS of stall", () => {
    const c = call({
      lastVerdict: "noticed",
      stalledSince: T0,
      lastOutputRevision: 10,
    });
    const a = tickCall(c, 10, "some tail", {
      now: T0 + 181_000,
      softS: 60,
      judgeS: 180,
    });
    expect(a.kind).toBe("judge");
    expect((a as { tail: string }).tail).toBe("some tail");
    expect(c.judgments).toBe(1);
  });

  it("re-judges only after the re-judge cadence and caps per call", () => {
    const c = call({
      lastVerdict: "HUNG",
      stalledSince: T0,
      lastOutputRevision: 10,
      judgments: 1,
      judgedAt: T0 + 181_000,
    });
    expect(
      tickCall(c, 10, "", { now: T0 + 240_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("none"); // too soon after the last judgment
    const a = tickCall(c, 10, "", {
      now: T0 + 500_000,
      softS: 60,
      judgeS: 180,
    });
    expect(a.kind).toBe("judge");
    expect(c.judgments).toBe(2);
    c.judgments = MAX_JUDGMENTS_PER_CALL;
    expect(
      tickCall(c, 10, "", { now: T0 + 900_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("notice");
  });
});

describe("parseVerdict", () => {
  it("reads the first VERDICT line and fails open", () => {
    expect(parseVerdict("VERDICT: HUNG\nIt waits on stdin.")).toEqual({
      alive: false,
      reason: "It waits on stdin.",
    });
    expect(parseVerdict("noise\nVERDICT: ALIVE\ncompiling links").alive).toBe(
      true,
    );
    expect(parseVerdict("garbage").alive).toBeNull();
    expect(parseVerdict("garbage").reason).toContain("unparseable");
  });

  it("marks a broken judge as NO-JUDGE, never as a liveness verdict", () => {
    const broken = parseVerdict("garbage");
    expect(broken.unparsed).toBe(true);
    expect(broken.reason).toContain("NO-JUDGE");
    // A real UNKNOWN is a verdict, not a parse failure.
    const unknown = parseVerdict("VERDICT: UNKNOWN no evidence");
    expect(unknown.alive).toBeNull();
    expect(unknown.unparsed).toBeUndefined();
    // Two verdict lines are ambiguous → NO-JUDGE.
    expect(parseVerdict("VERDICT: ALIVE\nVERDICT: HUNG").unparsed).toBe(true);
  });
});

describe("buildJudgePrompt", () => {
  it("carries command, timings, tail and the one-line contract", () => {
    const p = buildJudgePrompt("brievc build", 300, 200, "partial output\n");
    expect(p).toContain("brievc build");
    expect(p).toContain("300s");
    expect(p).toContain("200s");
    expect(p).toContain("VERDICT: ALIVE or VERDICT: HUNG");
    // The evidence sentence must not re-declare a verdict, or the strict parse
    // would reject every otherwise-valid answer (2026-11-15).
    expect(p).toContain("Never write the words ALIVE, HUNG or UNKNOWN again");
    expect(p).toContain("never started");
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

describe("output observation and judgment lifecycle", () => {
  it("reads real Pi content envelopes and bounded same-length rolling output", () => {
    const c = call();
    expect(toolOutputText({ content: [{ type: "text", text: "hello" }] })).toBe(
      "hello",
    );
    const a = "a".repeat(5000),
      b = "b".repeat(5000);
    expect(
      observeToolOutput(c, { content: [{ type: "text", text: a }] }, T0 + 1000),
    ).toBe(true);
    expect(
      observeToolOutput(c, { content: [{ type: "text", text: b }] }, T0 + 2000),
    ).toBe(true);
    expect(c.tail!.length).toBeLessThanOrEqual(1500);
    expect(c.lastActivityAt).toBe(T0 + 2000);
    expect(
      observeToolOutput(c, { content: [{ type: "text", text: b }] }, T0 + 3000),
    ).toBe(false);
    expect(
      observeToolOutput(
        c,
        { content: [{ type: "image", data: "ignored" }] },
        T0 + 4000,
      ),
    ).toBe(false);
  });

  it("timestamps early output and rearms notices and budget after genuine progress", () => {
    const c = call({ judgments: 3, lastVerdict: "UNKNOWN", judgedAt: T0 });
    observeToolOutput(c, "new output", T0 + 10_000);
    expect(c.judgments).toBe(0);
    expect(
      tickCall(c, c.lastOutputRevision, c.tail!, {
        now: T0 + 61_000,
        softS: 60,
        judgeS: 180,
      }).kind,
    ).toBe("none");
    expect(
      tickCall(c, c.lastOutputRevision, c.tail!, {
        now: T0 + 71_000,
        softS: 60,
        judgeS: 180,
      }).kind,
    ).toBe("notice");
  });

  it("rates limits unavailable/noticed attempts and blocks overlapping judgments", () => {
    const c = call({
      lastVerdict: "noticed",
      judgedAt: T0 + 181_000,
      judgments: 1,
    });
    expect(
      tickCall(c, 0, "", { now: T0 + 211_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("none");
    c.judging = true;
    expect(
      tickCall(c, 0, "", { now: T0 + 700_000, softS: 60, judgeS: 180 }).kind,
    ).toBe("none");
    expect(
      parseVerdict("VERDICT: UNKNOWN insufficient process evidence").alive,
    ).toBeNull();
  });
});
