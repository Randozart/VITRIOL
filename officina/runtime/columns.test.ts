import { describe, expect, it } from "vitest";
import { Columns, cutAnsi, visibleWidth } from "./columns.ts";

const mk = (lines: string[]) => ({ render: (_w: number) => lines, invalidate() {} });

describe("visibleWidth", () => {
  it("ignores ANSI escapes", () => {
    const line = "\x1b[38;2;255;215;0mLapis\x1b[0m";
    expect(visibleWidth(line)).toBe(5);
  });

  it("counts wide chars as 2", () => {
    expect(visibleWidth("日本")).toBe(4);
  });
});

describe("cutAnsi", () => {
  it("cuts to visible width while keeping color escapes", () => {
    const line = "\x1b[38;2;255;0;0mabcdef\x1b[0m";
    const cut = cutAnsi(line, 3);
    expect(cut).toContain("\x1b[38;2;255;0;0m");
    expect(cut.startsWith("\x1b[38;2;255;0;0mabc")).toBe(true);
    expect(cut.endsWith("\x1b[0m")).toBe(true);
  });
});

describe("Columns", () => {
  it("renders children side by side: slot pad + gap pad", () => {
    const cols = new Columns(
      [
        { component: mk(["ab"]), width: 4 },
        { component: mk(["cd"]), width: 4 },
      ],
      2,
    );
    expect(cols.render(10)).toEqual(["ab    cd  "]);
  });

  it("pads shorter columns and lets taller ones overflow", () => {
    const cols = new Columns(
      [
        { component: mk(["a", "b"]), width: 3 },
        { component: mk(["x"]), width: 3 },
      ],
      1,
    );
    expect(cols.render(8)).toEqual(["a   x  ", "b      "]);
  });

  it("flex shares split remaining width", () => {
    const cols = new Columns([
      { component: mk(["0123456789"]), width: { share: 0.7 } },
      { component: mk(["abcdefghij"]), width: { share: 0.3 } },
    ]);
    const lines = cols.render(10);
    expect(lines).toHaveLength(1);
    expect(lines[0]!.startsWith("012345")).toBe(true);
    expect(lines[0]!.includes("ab")).toBe(true);
  });

  it("cuts ANSI-colored overflow without breaking the row width", () => {
    const colored = {
      render: () => ["\x1b[38;2;0;255;255m0123456789\x1b[0m"],
      invalidate() {},
    };
    const cols = new Columns(
      [
        { component: colored, width: 6 },
        { component: mk(["z"]), width: 2 },
      ],
      1,
    );
    const row = cols.render(9)[0] ?? "";
    expect(row.trimEnd().endsWith("z")).toBe(true);
  });
});
