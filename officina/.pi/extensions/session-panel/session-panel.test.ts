import { describe, expect, it } from "vitest";
import { CONTENT_W, embDiv, modelMatchesLoaded, visibleLen } from "./index.ts";

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

describe("modelMatchesLoaded — pi model-id vs engine-loaded identity", () => {
  const alias = "Lapis Occultus";
  const path = "/home/randozart/Downloads/Qwen3.8-27B-Q3_K_M.gguf";

  it("accepts the alias", () => {
    expect(modelMatchesLoaded("Lapis Occultus", alias, path)).toBe(true);
  });

  it("accepts the loaded path", () => {
    expect(modelMatchesLoaded(path, alias, path)).toBe(true);
  });

  it("accepts the loaded FILENAME with extension (pi's natural id — 2026-09-04 hole)", () => {
    expect(modelMatchesLoaded("Qwen3.8-27B-Q3_K_M.gguf", alias, path)).toBe(true);
  });

  it("accepts the stripped basename", () => {
    expect(modelMatchesLoaded("Qwen3.8-27B-Q3_K_M", alias, path)).toBe(true);
  });

  it("rejects a genuine mismatch", () => {
    expect(modelMatchesLoaded("Qwen3.8-9B-Q8_0.gguf", alias, path)).toBe(false);
  });

  it("rejects an empty selection", () => {
    expect(modelMatchesLoaded("", alias, path)).toBe(false);
  });
});
