import { describe, expect, it } from "vitest";
import {
  activitySignature,
  continuationPrompt,
  decide,
  freshState,
  grindDisabled,
  hasOpenWork,
  maxMinutes,
  maxTurns,
  parseMarker,
  type DecideInput,
  type GrindState,
  type TaskSnapshot,
} from "./rules.ts";

const NOW = 1_700_000_000_000;

function state(over: Partial<GrindState> = {}): GrindState {
  return { ...freshState(NOW), ...over };
}

function base(over: Partial<DecideInput> = {}): DecideInput {
  return {
    mode: "grind",
    aborted: false,
    userDisarmed: false,
    queued: false,
    compacting: false,
    willRetry: false,
    assistantText: "did some work",
    tasks: null,
    verify: null,
    state: state(),
    now: NOW + 1000,
    maxTurns: 24,
    maxMinutes: 120,
    signature: "sig",
    ...over,
  };
}

const openTasks: TaskSnapshot = { pending: 2, inProgress: 1, open: ["a", "b", "c"] };

describe("parseMarker", () => {
  it("accepts DONE only on the final non-empty line", () => {
    expect(parseMarker("done with stuff\nGRIND: DONE").kind).toBe("done");
    expect(parseMarker("GRIND: DONE\nsome trailing chatter").kind).toBe("none");
    expect(parseMarker("I will write GRIND: DONE later").kind).toBe("none");
    expect(parseMarker("GRIND: done\n").kind).toBe("done");
  });

  it("parses BLOCKED with a reason", () => {
    const m = parseMarker("...\nGRIND: BLOCKED: needs the prod API key");
    expect(m).toEqual({ kind: "blocked", reason: "needs the prod API key" });
    expect(parseMarker("GRIND: BLOCKED")).toEqual({ kind: "blocked", reason: "no reason given" });
  });
});

describe("caps", () => {
  it("reads env caps with defaults and hard limits", () => {
    expect(maxTurns({} as NodeJS.ProcessEnv)).toBe(24);
    expect(maxTurns({ OFFICINA_GRIND_MAX_TURNS: "5" } as NodeJS.ProcessEnv)).toBe(5);
    expect(maxTurns({ OFFICINA_GRIND_MAX_TURNS: "9999" } as NodeJS.ProcessEnv)).toBe(100);
    expect(maxMinutes({} as NodeJS.ProcessEnv)).toBe(120);
    expect(grindDisabled({ OFFICINA_NO_GRIND: "1" } as NodeJS.ProcessEnv)).toBe(true);
    expect(grindDisabled({} as NodeJS.ProcessEnv)).toBe(false);
  });
});

describe("decide", () => {
  it("continues without a marker", () => {
    const d = decide(base());
    expect(d.action).toBe("continue");
    if (d.action === "continue") {
      expect(d.round).toBe(1);
      expect(d.prompt).toContain("GRIND round 1/24");
      expect(d.next.rounds).toBe(1);
    }
  });

  it("stops outside grind mode and when disarmed", () => {
    expect(decide(base({ mode: "build" }))).toMatchObject({ action: "stop", reason: "mode-changed" });
    expect(decide(base({ userDisarmed: true }))).toMatchObject({
      action: "stop",
      reason: "user-input",
    });
  });

  it("never re-prompts after an abort", () => {
    expect(decide(base({ aborted: true }))).toMatchObject({ action: "stop", reason: "aborted" });
  });

  it("stands down while queued, compacting, or retrying", () => {
    expect(decide(base({ queued: true }))).toMatchObject({ action: "stop", reason: "queued" });
    expect(decide(base({ compacting: true }))).toMatchObject({
      action: "stop",
      reason: "compacting",
    });
    expect(decide(base({ willRetry: true }))).toMatchObject({
      action: "stop",
      reason: "will-retry",
    });
  });

  it("accepts DONE only with an empty task list and green gates", () => {
    const ok = decide(base({ assistantText: "all done\nGRIND: DONE", tasks: null }));
    expect(ok).toMatchObject({ action: "stop", reason: "done" });
    const open = decide(
      base({ assistantText: "GRIND: DONE", tasks: openTasks, verify: { ran: true, failed: [] } }),
    );
    expect(open).toMatchObject({ action: "continue" });
    if (open.action === "continue") {
      expect(open.prompt).toContain("claimed GRIND: DONE");
      expect(open.prompt).toContain("3 task(s) still open");
    }
    const red = decide(
      base({ assistantText: "GRIND: DONE", verify: { ran: true, failed: ["test: 2 failures"] } }),
    );
    expect(red).toMatchObject({ action: "continue" });
  });

  it("accepts a rejected DONE claim after one reminder instead of fighting", () => {
    const d = decide(
      base({ assistantText: "GRIND: DONE", tasks: openTasks, state: state({ doneReminders: 1 }) }),
    );
    expect(d).toMatchObject({ action: "stop", reason: "done" });
  });

  it("stops on BLOCKED with the reason surfaced", () => {
    const d = decide(base({ assistantText: "GRIND: BLOCKED: needs credentials" }));
    expect(d).toMatchObject({ action: "stop", reason: "blocked" });
    if (d.action === "stop") expect(d.note).toContain("needs credentials");
  });

  it("enforces the turn and minute caps", () => {
    expect(decide(base({ state: state({ rounds: 24 }) }))).toMatchObject({
      action: "stop",
      reason: "cap-turns",
    });
    expect(
      decide(base({ state: state({ startedAt: NOW - 121 * 60_000 }) })),
    ).toMatchObject({ action: "stop", reason: "cap-minutes" });
  });

  it("stops a loop that makes no progress with a task list", () => {
    const sig = activitySignature(openTasks);
    const d = decide(
      base({
        tasks: openTasks,
        signature: sig,
        state: state({ lastSignature: sig, flatRounds: 2 }),
      }),
    );
    expect(d).toMatchObject({ action: "stop", reason: "no-progress" });
  });

  it("does not apply the flat guard without a task list", () => {
    const d = decide(base({ signature: "same", state: state({ lastSignature: "same", flatRounds: 9 }) }));
    expect(d.action).toBe("continue");
  });
});

describe("prompt + helpers", () => {
  it("lists open tasks and failing gates", () => {
    const p = continuationPrompt(2, 10, openTasks, { ran: true, failed: ["build: x"] });
    expect(p).toContain("GRIND round 2/10");
    expect(p).toContain("- a");
    expect(p).toContain("build: x");
    expect(p).toContain("GRIND: BLOCKED");
  });

  it("hasOpenWork counts pending and in-progress only", () => {
    expect(hasOpenWork(null)).toBe(false);
    expect(hasOpenWork({ pending: 0, inProgress: 0, open: [] })).toBe(false);
    expect(hasOpenWork({ pending: 1, inProgress: 0, open: ["a"] })).toBe(true);
  });
});
