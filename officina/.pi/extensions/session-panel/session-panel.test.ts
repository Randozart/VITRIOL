import { describe, expect, it } from "vitest";
import { CONTENT_W, embDiv, visibleLen } from "./index.ts";

describe("embDiv — sidebar divisors with embedded group headers", () => {
  it("renders exactly CONTENT_W visible columns", () => {
    for (const name of ["Engine", "Plans", "Session", "Commands"]) {
      expect(visibleLen(embDiv(name))).toBe(CONTENT_W);
    }
  });

  it("starts the rule with the group name inside it", () => {
    // sc() prepends the color code — check the ANSI-stripped form.
    const v = embDiv("Plans").replace(/\x1b\[[0-9;]*m/g, "");
    expect(v.startsWith("── Plans ")).toBe(true);
    expect(v.endsWith("─")).toBe(true);
  });

  it("wraps the line in MUTED color codes (single sc() span)", () => {
    const raw = embDiv("Session");
    expect(raw.startsWith("\x1b[")).toBe(true);
    expect(raw).toContain("Session");
  });

  it("handles long names without going over width", () => {
    const long = "An Absurdly Long Section Header Name";
    expect(visibleLen(embDiv(long))).toBe(CONTENT_W);
  });
});
