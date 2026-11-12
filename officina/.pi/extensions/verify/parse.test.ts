// Tests for verify gate-output parsers (pure).
import { describe, expect, it } from "vitest";
import { parseOutput } from "./parse.ts";

describe("tsc", () => {
  it("parses file(line,col): error TSxxxx", () => {
    const out = [
      "src/foo.ts(12,5): error TS2304: Cannot find name 'x'.",
      "src/foo.ts(20,1): error TS6133: 'y' is declared but never read.",
      "some informational line",
    ].join("\n");
    const diags = parseOutput("tsc", out);
    expect(diags).toHaveLength(2);
    expect(diags[0]).toMatchObject({ file: "src/foo.ts", line: 12, code: "TS2304" });
    expect(diags[1]!.code).toBe("TS6133");
  });
});

describe("cargo --message-format=short", () => {
  it("parses error[E0308] lines", () => {
    const out = "src/main.rs:42:7: error[E0308]: mismatched types\nhelp: run cargo fmt";
    const diags = parseOutput("cargo", out);
    expect(diags).toEqual([{ file: "src/main.rs", line: 42, code: "E0308", message: "mismatched types" }]);
  });
  it("bare errors without code", () => {
    const diags = parseOutput("cargo", "src/lib.rs:1:1: error: could not compile `x`");
    expect(diags[0]!.code).toBe("error");
  });
});

describe("rustfmt --check", () => {
  it("parses Diff in <file> at line N", () => {
    const diags = parseOutput("rustfmt", "Diff in ./src/foo.rs at line 10:\n@@ ...");
    expect(diags).toEqual([{ file: "./src/foo.rs", line: 10, code: "fmt", message: "needs rustfmt" }]);
  });
});

describe("ruff", () => {
  it("parses path:line:col: RULE msg", () => {
    const diags = parseOutput("ruff", "app/main.py:3:1: F401 `os` imported but unused");
    expect(diags[0]).toMatchObject({ file: "app/main.py", line: 3, code: "F401" });
  });
});

describe("vitest", () => {
  it("FAIL lines and stack frames, dedup", () => {
    const out = [
      " ✓ extensions/a.test.ts",
      "FAIL extensions/b.test.ts",
      "FAIL extensions/b.test.ts",
      " ❯ extensions/b.test.ts:12:3",
      " ❯ node_modules/vitest/x.js:1:1",
    ].join("\n");
    const diags = parseOutput("vitest", out);
    expect(diags.map((d) => d.file)).toEqual(["extensions/b.test.ts"]);
    expect(diags[0]!.line).toBe(12); // stack frame wins (later push? no — first push kept)
  });
  it("frame wins over FAIL line for the same file", () => {
    const diags = parseOutput("vitest", "FAIL t.test.ts\n ❯ t.test.ts:5:1");
    expect(diags).toHaveLength(1);
    expect(diags[0]).toMatchObject({ file: "t.test.ts", line: 5 });
  });
  it("FAIL line survives when no frame exists", () => {
    const diags = parseOutput("vitest", "FAIL extensions/solo.test.ts");
    expect(diags).toEqual([{ file: "extensions/solo.test.ts", line: null, code: "test", message: "failing tests" }]);
  });
});

describe("pytest", () => {
  it("FAILED short-summary lines", () => {
    const diags = parseOutput("pytest", "FAILED tests/test_x.py::test_y - AssertionError: 1 != 2\n1 failed");
    expect(diags[0]).toMatchObject({ file: "tests/test_x.py", code: "test", message: "AssertionError: 1 != 2" });
  });
});

describe("go", () => {
  it("file:line:col: message", () => {
    const diags = parseOutput("go", "./pkg/a.go:10:2: undefined: Foo");
    expect(diags[0]).toMatchObject({ file: "./pkg/a.go", line: 10, code: "go" });
  });
});

describe("generic", () => {
  it("emits nothing (gate-level fallback lives in runner)", () => {
    expect(parseOutput("generic", "anything at all")).toEqual([]);
  });
});
