// mode-return table tests (2026-11-21). I1/I2 pinned here; I7 (persistence
// via the facts file) is pinned in the agent-mode wiring tests.
import { describe, expect, it } from "vitest";
import {
  bounceTarget,
  isInterruptMode,
  isWorkMode,
  nextReturnTo,
} from "./mode-return.ts";

describe("mode-return", () => {
  it("classifies modes", () => {
    expect(isWorkMode("pipeline")).toBe(true);
    expect(isWorkMode("research")).toBe(true);
    expect(isWorkMode("grind")).toBe(true);
    expect(isWorkMode("steer")).toBe(false);
    expect(isInterruptMode("inquire")).toBe(true);
    expect(isInterruptMode("build")).toBe(false);
    expect(isWorkMode(null)).toBe(false);
  });

  // I2: entering the interrupt pair FROM a work mode records the origin.
  it("records origin when a work mode switches to steer/inquire", () => {
    expect(nextReturnTo("steer", "grind", null)).toBe("grind");
    expect(nextReturnTo("inquire", "pipeline", null)).toBe("pipeline");
    expect(nextReturnTo("steer", "research", "stale")).toBe("research");
  });

  // I2: steer↔inquire keeps the existing origin (the interloper pair shares it).
  it("keeps the origin across the steer↔inquire pair", () => {
    expect(nextReturnTo("inquire", "steer", "grind")).toBe("grind");
    expect(nextReturnTo("steer", "inquire", "pipeline")).toBe("pipeline");
    // From an interrupt mode with no origin: still nothing.
    expect(nextReturnTo("inquire", "steer", null)).toBeNull();
  });

  // I1: the operator picking a work mode clears returnTo — bounce-back must
  // never fight a manual pick.
  it("clears returnTo when the operator picks a work mode", () => {
    expect(nextReturnTo("grind", "steer", "grind")).toBeNull();
    expect(nextReturnTo("build", "inquire", "pipeline")).toBeNull();
  });

  // I1: bounce-back requires a live returnTo.
  it("bounce only fires in interrupt modes with a live origin", () => {
    expect(bounceTarget("steer", "grind")).toBe("grind");
    expect(bounceTarget("inquire", "pipeline")).toBe("pipeline");
    expect(bounceTarget("steer", null)).toBeNull();
    expect(bounceTarget("grind", "grind")).toBeNull();
    // A stale/garbage origin is not a bounce target.
    expect(bounceTarget("steer", "bogus")).toBeNull();
  });
});
