import { describe, expect, it } from "vitest";
import { formatterFor, renderFormatNotice, type Availability } from "./format.ts";

const all: Availability = () => true;
const none: Availability = () => false;

describe("formatterFor", () => {
  it("picks ruff for python when available", () => {
    expect(formatterFor("a.py", all)?.argv).toEqual(["ruff", "format", "a.py"]);
  });
  it("falls back to black", () => {
    expect(formatterFor("a.py", (c) => c === "black")?.argv).toEqual(["black", "--quiet", "a.py"]);
  });
  it("returns null when nothing is installed", () => {
    expect(formatterFor("a.py", none)).toBeNull();
  });
  it("picks prettier for js/ts/json/css", () => {
    for (const f of ["a.ts", "a.js", "a.mjs", "a.cjs", "a.tsx", "a.json", "a.css"]) {
      expect(formatterFor(f, all)?.label).toBe("prettier");
    }
  });
  it("picks gofmt / rustfmt / clang-format", () => {
    expect(formatterFor("a.go", all)?.label).toBe("gofmt");
    expect(formatterFor("a.rs", all)?.label).toBe("rustfmt");
    expect(formatterFor("a.cpp", all)?.label).toBe("clang-format");
  });
  it("returns null for unknown extensions", () => {
    expect(formatterFor("a.txt", all)).toBeNull();
    expect(formatterFor("a.md", all)).toBeNull();
  });
});

describe("renderFormatNotice", () => {
  it("is empty when nothing changed", () => {
    expect(renderFormatNotice([])).toBe("");
  });
  it("lists files and labels", () => {
    const out = renderFormatNotice([
      { file: "a.ts", label: "prettier" },
      { file: "b.py", label: "ruff format" },
    ]);
    expect(out).toContain("2 file(s)");
    expect(out).toContain("prettier, ruff format");
    expect(out).toContain("a.ts");
    expect(out).toContain("ON-DISK content is canonical");
  });
});
