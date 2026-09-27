// Tests for at-mention pure suggestion logic (parity plan phase 1.2).
import { describe, expect, it } from "vitest";
import { atTokenAt, fuzzyScore, mergeSuggestions, type CatalogEntry } from "./suggest.ts";

const entry = (label: string, value = label, description?: string): CatalogEntry => ({
  id: `t:${value}`,
  value,
  label,
  description,
  dir: false,
});

describe("atTokenAt", () => {
  it("finds a bare @ at end of line", () => {
    expect(atTokenAt("look @", 6)).toEqual({ raw: "@", query: "" });
  });
  it("finds an @token at line start", () => {
    expect(atTokenAt("@sc", 3)).toEqual({ raw: "@sc", query: "sc" });
  });
  it("finds an @token after whitespace", () => {
    expect(atTokenAt("read @.officina/SC", 18)).toEqual({ raw: "@.officina/SC", query: ".officina/SC" });
  });
  it("rejects mid-word @", () => {
    expect(atTokenAt("mail user@example", 17)).toBeNull();
  });
  it("rejects when cursor is before the token", () => {
    expect(atTokenAt("@sc tail", 4)).toBeNull();
  });
  it("rejects no-@ lines", () => {
    expect(atTokenAt("plain text", 10)).toBeNull();
  });
});

describe("fuzzyScore", () => {
  it("empty query matches everything maximally", () => {
    expect(fuzzyScore("SCRATCHPAD.md", "")).toBe(1);
  });
  it("contiguous match beats scattered", () => {
    expect(fuzzyScore("scratchpad", "scr")).toBeGreaterThan(fuzzyScore("s-p-a-t-c-h-r", "scr"));
  });
  it("boundary hit scores", () => {
    expect(fuzzyScore("session-panel", "pan")).toBeGreaterThan(0);
  });
  it("no match is 0", () => {
    expect(fuzzyScore("scratchpad", "zzz")).toBe(0);
  });
  it("is case-insensitive", () => {
    expect(fuzzyScore("MEMORY.md", "memo")).toBeGreaterThan(0);
  });
});

describe("mergeSuggestions", () => {
  const cat = [
    entry("SCRATCHPAD", ".officina/SCRATCHPAD.md", "scratchpad"),
    entry("auth-jwt", "/opt/officina/skills/knowledge/auth-jwt.md", "knowledge card"),
  ];
  const token = (line: string, col = line.length) => (l: string, c: number) => atTokenAt(l, c ?? col);

  it("appends extras after base items with the @ prefix", () => {
    const base = { items: [{ value: "src/auth.ts", label: "src/auth.ts" }], prefix: "@au" };
    const out = mergeSuggestions(base, cat, ["read @au"], 0, 9, atTokenAt);
    expect(out).not.toBeNull();
    expect(out!.prefix).toBe("@au");
    expect(out!.items[0].value).toBe("src/auth.ts");
    expect(out!.items.filter((i) => i.officina).map((i) => i.label)).toContain("auth-jwt");
  });

  it("returns extras alone when base has none but is at an @", () => {
    const out = mergeSuggestions(null, cat, ["@"], 0, 1, atTokenAt);
    expect(out).not.toBeNull();
    expect(out!.prefix).toBe("@");
    expect(out!.items.length).toBe(2);
  });

  it("passes the base through untouched away from @", () => {
    const base = { items: [{ value: "x", label: "x" }], prefix: "x" };
    expect(mergeSuggestions(base, cat, ["plain"], 0, 5, atTokenAt)).toEqual(base);
  });

  it("returns null at a @ with no matches at all", () => {
    expect(mergeSuggestions(null, cat, ["@qqq"], 0, 4, atTokenAt)).toBeNull();
  });

  it("dedupes base values", () => {
    const base = { items: [{ value: ".officina/SCRATCHPAD.md", label: "SCRATCHPAD" }], prefix: "@sc" };
    const out = mergeSuggestions(base, cat, ["@sc"], 0, 3, atTokenAt);
    expect(out!.items.filter((i) => i.value === ".officina/SCRATCHPAD.md")).toHaveLength(1);
  });

  it("caps extras", () => {
    const many = Array.from({ length: 20 }, (_, i) => entry(`card${i}`, `card${i}.md`, "tool card"));
    const out = mergeSuggestions(null, many, ["@card"], 0, 5, atTokenAt);
    expect(out!.items.filter((i) => i.officina).length).toBeLessThanOrEqual(6);
  });
});
