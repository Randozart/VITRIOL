import { describe, expect, it } from "vitest";
import {
  DEFAULT_OPTIONS,
  filterSessionsByCwd,
  normalizeOpenCodeSession,
  parseSessionList,
  sessionDirForCwd,
  toSessionJsonl,
} from "./opencode-import-lib.mjs";

const source = {
  session: { id: "ses_test", directory: "/home/u/proj", model: { providerID: "opencode-go", id: "m" } },
  messages: [
    { id: "m1", time_created: 1000, data: { role: "user", time: { created: 1000 } } },
    { id: "m2", time_created: 1100, data: { role: "assistant", time: { created: 1100 }, modelID: "m", providerID: "opencode-go", finish: "tool-calls", tokens: { input: 10, output: 2, reasoning: 1, total: 13, cache: { read: 5, write: 0 } }, cost: 0.01 } },
    { id: "m3", time_created: 1200, data: { role: "user", time: { created: 1200 } } },
    { id: "m4", time_created: 1300, data: { role: "assistant", time: { created: 1300 }, modelID: "m", providerID: "opencode-go", finish: "stop" } },
  ],
  parts: [
    { id: "p1", message_id: "m1", time_created: 1000, data: { type: "text", text: "Please fix the bug" } },
    { id: "p2", message_id: "m2", time_created: 1100, data: { type: "reasoning", text: "Need to inspect files" } },
    { id: "p3", message_id: "m2", time_created: 1101, data: { type: "tool", tool: "bash", callID: "call1", state: { status: "completed", input: { command: "ls" }, output: "a\nb\n", time: { end: 1102 } } } },
    { id: "p4", message_id: "m3", time_created: 1200, data: { type: "text", text: "Now finish" } },
    { id: "p5", message_id: "m4", time_created: 1300, data: { type: "text", text: "Done" } },
  ],
};

describe("session list helpers", () => {
  it("parses JSON and filters by cwd", () => {
    const sessions = parseSessionList(JSON.stringify([
      { id: "a", directory: "/home/u/proj", updated: 2 },
      { id: "b", directory: "/home/u/other", updated: 3 },
    ]));
    expect(filterSessionsByCwd(sessions, "/home/u/proj").map((s) => s.id)).toEqual(["a"]);
  });

  it("encodes the pi session directory", () => {
    expect(sessionDirForCwd("/home/u/proj", "/sessions")).toBe("/sessions/--home-u-proj--");
  });
});

describe("normalizeOpenCodeSession", () => {
  it("normalizes text, reasoning, tool calls, and results", () => {
    const result = normalizeOpenCodeSession(source, DEFAULT_OPTIONS);
    expect(result.blocks).toHaveLength(4);
    expect(result.blocks[0]).toMatchObject({ kind: "user", text: "Please fix the bug" });
    expect(result.blocks[1].content).toEqual([
      { type: "thinking", thinking: "Need to inspect files" },
      { type: "toolCall", id: "call1", name: "bash", arguments: { command: "ls" } },
    ]);
    expect(result.blocks[1].toolResults[0]).toMatchObject({ toolName: "bash", output: "a\nb\n" });
    expect(result.blocks[2]).toMatchObject({ kind: "user", text: "Now finish" });
    expect(result.blocks[3]).toMatchObject({ kind: "assistant", content: [{ type: "text", text: "Done" }] });
  });

  it("caps tool output and reasoning", () => {
    const big = {
      ...source,
      parts: source.parts.map((p) => p.id === "p2"
        ? { ...p, data: { type: "reasoning", text: "x".repeat(2000) } }
        : p.id === "p3"
          ? { ...p, data: { ...p.data, state: { ...p.data.state, output: "y".repeat(2000) } } }
          : p),
    };
    const result = normalizeOpenCodeSession(big, { maxReasoningChars: 50, maxToolChars: 60 });
    expect(result.blocks[1].content[0].thinking.length).toBeLessThan(100);
    expect(result.blocks[1].toolResults[0].output.length).toBeLessThan(120);
  });

  it("imports every block by default", () => {
    const result = normalizeOpenCodeSession(source, DEFAULT_OPTIONS);
    expect(result.blocks).toHaveLength(4);
  });

  it("keeps only the last N blocks when explicitly asked", () => {
    const result = normalizeOpenCodeSession(source, { last: 1, maxChars: 10_000 });
    expect(result.blocks).toHaveLength(2);
    expect(result.blocks[0].kind).toBe("user");
    expect(result.blocks[1].kind).toBe("assistant");
  });
});

describe("toSessionJsonl", () => {
  it("emits a v3 session with a linked parent chain", () => {
    const normalized = normalizeOpenCodeSession(source, DEFAULT_OPTIONS);
    const text = toSessionJsonl(source, normalized, { cwd: "/home/u/proj" });
    const entries = text.trim().split("\n").map((line) => JSON.parse(line));
    expect(entries[0]).toMatchObject({ type: "session", version: 3, cwd: "/home/u/proj" });
    expect(entries.filter((e) => e.type === "message")).toHaveLength(5);
    for (let i = 2; i < entries.length; i += 1) {
      expect(entries[i].parentId).toBe(entries[i - 1].id);
    }
    expect(text).toContain('"type":"toolCall"');
    expect(text).toContain('"role":"toolResult"');
  });
});
