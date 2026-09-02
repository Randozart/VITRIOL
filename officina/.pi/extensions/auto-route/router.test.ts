import { describe, it, expect } from "vitest";
import { effectiveComplexity, resolveMode, resolveThreshold, route } from "./router.ts";

describe("resolveThreshold", () => {
  it("defaults to 0.5", () => {
    expect(resolveThreshold(undefined)).toBe(0.5);
    expect(resolveThreshold("")).toBe(0.5);
    expect(resolveThreshold("junk")).toBe(0.5);
  });
  it("clamps into [0,1]", () => {
    expect(resolveThreshold("0")).toBe(0);
    expect(resolveThreshold("1")).toBe(1);
    expect(resolveThreshold("-3")).toBe(0);
    expect(resolveThreshold("7")).toBe(1);
    expect(resolveThreshold("0.25")).toBe(0.25);
  });
});

describe("resolveMode", () => {
  it("defaults to suggest; passes valid modes through", () => {
    expect(resolveMode(undefined)).toBe("suggest");
    expect(resolveMode("weird")).toBe("suggest");
    expect(resolveMode("auto")).toBe("auto");
    expect(resolveMode("off")).toBe("off");
  });
});

describe("effectiveComplexity", () => {
  it("dampens complexity as threshold rises", () => {
    expect(effectiveComplexity(0.9, 0)).toBe(0.9);
    expect(effectiveComplexity(0.9, 1)).toBeCloseTo(0.3);
    expect(effectiveComplexity(0.9, 0.5)).toBeCloseTo(0.9 * 0.5 + 0.15);
  });
});

describe("route", () => {
  it("confidential never leaves the small local model", () => {
    expect(route(1.0, "confidential", 0).tier).toBe("local-sm");
  });
  it("sensitive stays local but may use the flagship", () => {
    expect(route(0.9, "sensitive", 0).tier).toBe("local-lg");
    expect(route(0.2, "sensitive", 0.5).tier).toBe("local-sm");
  });
  it("safe + high complexity escalates to cloud", () => {
    expect(route(0.9, "safe", 0.5).tier).toBe("cloud");
  });
  it("safe + medium complexity uses the flagship", () => {
    expect(route(0.5, "safe", 0.5).tier).toBe("local-lg");
  });
  it("safe + low complexity uses the small model", () => {
    expect(route(0.15, "safe", 0.5).tier).toBe("local-sm");
  });
  it("threshold 1.0 keeps even complex safe work local", () => {
    expect(route(0.9, "safe", 1.0).tier).not.toBe("cloud");
  });
  it("threshold 0.0 escalates eagerly", () => {
    expect(route(0.35, "safe", 0.0).tier).toBe("cloud");
  });
  it("decision carries reason and effective score", () => {
    const d = route(0.9, "safe", 0.5);
    expect(d.reason).toContain("complexity");
    expect(d.effective).toBeGreaterThan(0);
  });
});
