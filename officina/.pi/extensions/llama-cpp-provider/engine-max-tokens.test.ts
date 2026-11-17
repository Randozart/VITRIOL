import { describe, expect, it } from "vitest";
import { engineMaxTokens } from "./config.ts";

// Regression (2026-11-17): pi's compaction summary runs on the engine alias and
// writes thousands of tokens. The alias was registered with
// fillModelDefaults' 4096 default, so a 134k-token conversation summarised into
// "generation hit the token cap and the summary is incomplete" — every local
// compaction failed. The alias must advertise a real output budget.
describe("engineMaxTokens", () => {
  it("gives a deep-context engine room for a summary", () => {
    expect(engineMaxTokens(262144)).toBe(16384);
    expect(engineMaxTokens(131072)).toBe(16384);
  });

  it("never goes below the historical default and never past the cap", () => {
    expect(engineMaxTokens(32768)).toBe(16384); // the floor of the "big window" class
    expect(engineMaxTokens(4096)).toBe(4096); // tiny context keeps the historical floor
    expect(engineMaxTokens(1_000_000)).toBe(16384); // capped
  });

  it("survives an unknown context window", () => {
    // Alias registration can run before the window probe lands; a summary must
    // still have a usable budget rather than 0 or NaN.
    expect(engineMaxTokens(undefined)).toBe(4096);
    expect(engineMaxTokens(Number.NaN)).toBe(4096);
  });
});
