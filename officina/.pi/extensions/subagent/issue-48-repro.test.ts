import { describe, it, expect } from "vitest";
import { SubCoderTracker } from "./tracker.ts";

// Live regression for issue #48 — the user's reported crash shape.
// pi-tui throws when any rendered widget line is wider than the terminal.
// Before the fix, the sub-coder tracker fed an unbounded errorMessage straight
// into a widget row; a ~167-char error at terminal width 184 produced a
// 248-char visible line and crashed the session. This test simulates the
// exact scenario and asserts every emitted line fits.

describe("issue #48 — tracker doesn't overflow terminal width", () => {
  it("caps a failed sub-coder's row to the terminal width", () => {
    const orig = (process.stdout as any).columns;
    (process.stdout as any).columns = 184;
    try {
      const captured: string[] = [];
      const ctx = {
        hasUI: true,
        ui: {
          setWidget: (_k: string, lines: string[] | undefined) => {
            if (lines) captured.push(...lines);
          },
        },
      };
      const tracker = new SubCoderTracker(ctx, { key: "t", totalSince: Date.now() - 47000 });
      tracker.begin([{ id: "a", label: "deep-explorer-research" }]);
      const longErr =
        "child process exited with non-zero code 1: " +
        "Error: provider \"llamacpp\" — failed to reach " +
        "http://127.0.0.1:8888/v1/chat/completions: ECONNREFUSED (transport error 503 after 3 retries)";
      tracker.update([
        {
          id: "a",
          label: "deep-explorer-research",
          task: "",
          exitCode: 1,
          errorMessage: longErr,
          report: "",
          messages: [],
          stderr: "",
          usage: { input: 0, output: 0, cost: 0, turns: 0, contextTokens: 0 },
        },
      ]);
      tracker.end();
      expect(captured.length).toBeGreaterThan(0);
      const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
      const widths = captured.map((l) => stripAnsi(l).length);
      const max = Math.max(...widths);
      expect(max).toBeLessThanOrEqual(184);
    } finally {
      (process.stdout as any).columns = orig;
    }
  });

  it("survives narrower terminals (~80 cols)", () => {
    const orig = (process.stdout as any).columns;
    (process.stdout as any).columns = 80;
    try {
      const captured: string[] = [];
      const ctx = {
        hasUI: true,
        ui: { setWidget: (_k: string, lines: string[] | undefined) => { if (lines) captured.push(...lines); } },
      };
      const tracker = new SubCoderTracker(ctx);
      tracker.begin([{ id: "a", label: "x" }, { id: "b", label: "y" }]);
      tracker.update([
        { id: "a", label: "x", task: "", exitCode: 0, report: "ok", messages: [], stderr: "", usage: { input: 0, output: 0, cost: 0, turns: 0, contextTokens: 0 } },
        { id: "b", label: "y", task: "", exitCode: 1, errorMessage: "x".repeat(500), report: "", messages: [], stderr: "", usage: { input: 0, output: 0, cost: 0, turns: 0, contextTokens: 0 } },
      ]);
      tracker.end();
      const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
      const max = Math.max(...captured.map((l) => stripAnsi(l).length));
      expect(max).toBeLessThanOrEqual(80);
    } finally {
      (process.stdout as any).columns = orig;
    }
  });
});
