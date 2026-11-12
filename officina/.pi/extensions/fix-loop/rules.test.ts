// fix-loop rules (exec-infra stage 7, I7: explicit, capped, single-flight).
import { describe, expect, it } from "vitest";
import type { LoopState } from "./rules.ts";
import {
  autocommitEnabled,
  bumpRun,
  canRun,
  dayKey,
  fixPrompt,
  MAX_PER_DAY,
  planRounds,
} from "./rules.ts";

describe("planRounds", () => {
  it("defaults to 3, clamps to 1..10, 0 when killed", () => {
    expect(planRounds(undefined)).toBe(3);
    expect(planRounds("")).toBe(3);
    expect(planRounds("5")).toBe(5);
    expect(planRounds("0")).toBe(1);
    expect(planRounds("-4")).toBe(1);
    expect(planRounds("99")).toBe(10);
    expect(planRounds("3", { OFFICINA_NO_FIX_LOOP: "1" } as NodeJS.ProcessEnv)).toBe(0);
  });
});

describe("day cap", () => {
  it("resets on a new day and refuses at the cap", () => {
    const today = dayKey();
    expect(canRun(null)).toEqual({ ok: true, reason: "" });
    expect(canRun({ version: 1, day: "1999-01-01", runsToday: MAX_PER_DAY })).toEqual({
      ok: true,
      reason: "",
    });
    const capped = canRun({ version: 1, day: today, runsToday: MAX_PER_DAY });
    expect(capped.ok).toBe(false);
    expect(capped.reason).toContain("day-capped");
    expect(canRun({ version: 1, day: today, runsToday: MAX_PER_DAY - 1 }).ok).toBe(true);
    expect(canRun({ version: 2, day: today, runsToday: 99 } as unknown as LoopState).ok).toBe(true); // unknown version → fresh
  });

  it("bumpRun counts per day", () => {
    expect(bumpRun(null)).toEqual({ version: 1, day: dayKey(), runsToday: 1 });
    expect(bumpRun({ version: 1, day: dayKey(), runsToday: 2 }).runsToday).toBe(3);
    expect(bumpRun({ version: 1, day: "1999-01-01", runsToday: 5 }).runsToday).toBe(1);
  });
});

describe("fixPrompt", () => {
  it("names gates, files and the round budget", () => {
    const text = fixPrompt(
      [
        {
          gateId: "cargo-test",
          gateLabel: "cargo test",
          diag: { file: "src/x.rs", message: "assertion failed: foo()".repeat(30) },
        },
      ],
      2,
      3,
    );
    expect(text).toContain("round 2/3");
    expect(text).toContain("- cargo-test (cargo test) — src/x.rs");
    expect(text).toContain("assertion failed");
    expect(text.length).toBeLessThan(2_000);
    expect(text).toContain("STOP and explain");
  });
});

describe("autocommitEnabled", () => {
  it("is default-off and env-on (I7)", () => {
    expect(autocommitEnabled({} as NodeJS.ProcessEnv)).toBe(false);
    expect(autocommitEnabled({ OFFICINA_FIXLOOP_AUTOCOMMIT: "1" } as NodeJS.ProcessEnv)).toBe(true);
    expect(autocommitEnabled({ OFFICINA_FIXLOOP_AUTOCOMMIT: "0" } as NodeJS.ProcessEnv)).toBe(false);
  });
});
