// verify — gate-output parsers (pure, unit-tested).
//
// Each parser turns a gate's combined stdout+stderr into file-scoped
// diagnostics. Parsers that find nothing on a FAILING gate fall back to a
// gate-level diagnostic (file: null) in runner.ts — half a report beats a
// false file attribution.

import type { ParserId } from "./gates.ts";

export interface Diag {
  /** Absolute file path (resolved against the gate dir by the runner), or null for gate-level. */
  file: string | null;
  line: number | null;
  /** Stable code: TS2304, E0308, fmt, ruff-rule, gate, … */
  code: string;
  message: string;
}

export function parseOutput(parser: ParserId, output: string): Diag[] {
  if (parser === "vitest") return parseVitest(output);
  const out: Diag[] = [];
  const seen = new Set<string>();
  const push = (d: Diag): void => {
    const key = `${d.file ?? "*"}:${d.line ?? 0}:${d.code}:${d.message}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push(d);
  };

  for (const line of output.split("\n")) {
    let m: RegExpMatchArray | null = null;
    switch (parser) {
      case "tsc":
        // path(line,col): error TS2304: Cannot find name 'x'.
        m = line.match(/^(.+?)\((\d+),\d+\): error (TS\d+): (.+)$/);
        if (m) push({ file: m[1]!, line: Number(m[2]), code: m[3]!, message: m[4]!.trim() });
        break;
      case "cargo":
        // --message-format=short: path:line:col: error[E0308]: mismatched types
        m = line.match(/^(.+?):(\d+):\d+: error(?:\[E(\d+)\])?: (.+)$/);
        if (m) push({ file: m[1]!, line: Number(m[2]), code: m[3] ? `E${m[3]}` : "error", message: m[4]!.trim() });
        break;
      case "rustfmt":
        // rustfmt --check: Diff in ./src/foo.rs at line 12:
        m = line.match(/^Diff in (\S+) at line (\d+)/);
        if (m) push({ file: m[1]!, line: Number(m[2]), code: "fmt", message: "needs rustfmt" });
        break;
      case "ruff":
        // path:line:col: RULE123 message
        m = line.match(/^(.+?):(\d+):\d+: ([A-Z][A-Z0-9]+) (.+)$/);
        if (m) push({ file: m[1]!, line: Number(m[2]), code: m[3]!, message: m[4]!.trim() });
        break;
      case "pytest":
        // short summary: FAILED tests/test_x.py::test_y - AssertionError
        m = line.match(/^FAILED (\S+?)(?:::(\S+))?(?: - (.*))?$/);
        if (m) push({ file: m[1]!, line: null, code: "test", message: (m[3] ?? m[2] ?? "failing test").slice(0, 200) });
        break;
      case "go":
        // ./file.go:12:3: message
        m = line.match(/^(.+?\.go):(\d+):\d+: (.+)$/);
        if (m) push({ file: m[1]!, line: Number(m[2]), code: "go", message: m[3]!.trim() });
        break;
      case "generic":
        break;
    }
  }
  return out;
}

/**
 * Vitest needs a two-pass merge: stack frames ("❯ path:12:3") carry the
 * precise location; bare "FAIL path" lines cover suites whose frames were
 * swallowed by reporter truncation. Frame wins for the same file.
 */
function parseVitest(output: string): Diag[] {
  const frames = new Map<string, number>();
  const failFiles: string[] = [];
  for (const line of output.split("\n")) {
    let m = line.match(/^\s*❯ (.+?):(\d+):\d+/);
    if (m && !m[1]!.includes("node_modules")) {
      frames.set(m[1]!, Number(m[2]));
      continue;
    }
    m = line.match(/^FAIL\s+(\S+)/);
    if (m && !m[1]!.startsWith("⎯")) failFiles.push(m[1]!);
  }
  const out: Diag[] = [];
  for (const [file, line] of frames) out.push({ file, line, code: "test", message: "failing test" });
  for (const file of failFiles) {
    if (!frames.has(file)) out.push({ file, line: null, code: "test", message: "failing tests" });
  }
  return out;
}
