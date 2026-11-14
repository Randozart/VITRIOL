import { describe, expect, it } from "vitest";
import {
  DEFAULT_OPTIONS,
  FULL_OPTIONS,
  compactSessionText,
  validateSessionText,
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

describe("default import budget", () => {
  function bigSource(pairs = 400) {
    const messages = [];
    const parts = [];
    for (let i = 0; i < pairs; i += 1) {
      messages.push({ id: `u${i}`, time_created: i * 10, data: { role: "user", time: { created: i * 10 } } });
      messages.push({ id: `a${i}`, time_created: i * 10 + 1, data: { role: "assistant", time: { created: i * 10 + 1 }, modelID: "m", providerID: "x", finish: "stop" } });
      parts.push({ id: `p${i}`, message_id: `u${i}`, data: { type: "text", text: `user prompt ${i} ${"x".repeat(500)}` } });
      parts.push({ id: `q${i}`, message_id: `a${i}`, data: { type: "text", text: `answer ${i} ${"y".repeat(500)}` } });
    }
    return { session: { id: "s", directory: "/tmp/p" }, messages, parts };
  }

  it("keeps a manageable window and digests the omitted history", () => {
    const src = bigSource();
    const n = normalizeOpenCodeSession(src, DEFAULT_OPTIONS);
    expect(DEFAULT_OPTIONS.maxChars).toBe(160_000);
    expect(n.blocks.length).toBeLessThan(src.messages.length);
    expect(n.stats.omittedBlocks).toBeGreaterThan(0);
    expect(n.stats.chars).toBeLessThanOrEqual(DEFAULT_OPTIONS.maxChars);
    expect(n.blocks[0].kind).toBe("user");
    expect(n.digest).toContain("omitted to keep the context manageable");
    expect(n.digest).toContain("user prompt 0");
    expect(n.digest).toContain("--full");
  });

  it("--full restores everything with no digest", () => {
    const src = bigSource(40);
    const n = normalizeOpenCodeSession(src, FULL_OPTIONS);
    expect(FULL_OPTIONS.maxChars).toBe(0);
    expect(n.blocks).toHaveLength(80);
    expect(n.digest).toBe("");
    expect(n.stats.omittedBlocks).toBe(0);
  });

  it("leaves small sessions untouched", () => {
    const n = normalizeOpenCodeSession(source, DEFAULT_OPTIONS);
    expect(n.blocks).toHaveLength(4);
    expect(n.digest).toBe("");
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

  it("embeds the digest as a hidden context entry", () => {
    const messages = [];
    const parts = [];
    for (let i = 0; i < 400; i += 1) {
      messages.push({ id: `u${i}`, time_created: i * 10, data: { role: "user", time: { created: i * 10 } } });
      messages.push({ id: `a${i}`, time_created: i * 10 + 1, data: { role: "assistant", time: { created: i * 10 + 1 }, modelID: "m", providerID: "x", finish: "stop" } });
      parts.push({ id: `p${i}`, message_id: `u${i}`, data: { type: "text", text: `p${i} ${"x".repeat(500)}` } });
      parts.push({ id: `q${i}`, message_id: `a${i}`, data: { type: "text", text: `a${i} ${"y".repeat(500)}` } });
    }
    const big = { session: { id: "s", directory: "/tmp/p" }, messages, parts };
    const normalized = normalizeOpenCodeSession(big, DEFAULT_OPTIONS);
    const entries = toSessionJsonl(big, normalized, { cwd: "/tmp/p" })
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line));
    expect(entries[0].type).toBe("session");
    expect(entries[1]).toMatchObject({
      type: "custom_message",
      customType: "opencode-import",
      display: false,
      parentId: null,
    });
    expect(entries[1].content).toContain("omitted");
    // The chain continues from the digest entry.
    expect(entries[2].parentId).toBe(entries[1].id);
  });
});

function sessionWithTools(pairs = 60) {
  const messages = [];
  const parts = [];
  for (let i = 0; i < pairs; i += 1) {
    messages.push({ id: `u${i}`, time_created: i * 10, data: { role: "user", time: { created: i * 10 } } });
    messages.push({ id: `a${i}`, time_created: i * 10 + 1, data: { role: "assistant", time: { created: i * 10 + 1 }, modelID: "m", providerID: "x", finish: "tool-calls" } });
    messages.push({ id: `b${i}`, time_created: i * 10 + 2, data: { role: "assistant", time: { created: i * 10 + 2 }, modelID: "m", providerID: "x", finish: "stop" } });
    parts.push({ id: `p${i}`, message_id: `u${i}`, data: { type: "text", text: `request ${i} ${"x".repeat(300)}` } });
    parts.push({
      id: `q${i}`,
      message_id: `a${i}`,
      data: { type: "tool", tool: "bash", callID: `call${i}`, state: { status: "completed", input: { command: "ls" }, output: `out ${i}`, time: { end: i * 10 + 2 } } },
    });
    parts.push({ id: `r${i}`, message_id: `b${i}`, data: { type: "text", text: `done ${i} ${"y".repeat(300)}` } });
  }
  return { session: { id: "s", directory: "/tmp/p" }, messages, parts };
}

describe("compactSessionText + validateSessionText", () => {
  it("re-budgets an oversized session, digests the rest, keeps the chain valid", () => {
    const src = sessionWithTools(60);
    const normalized = normalizeOpenCodeSession(src, FULL_OPTIONS);
    const original = toSessionJsonl(src, normalized, { cwd: "/tmp/p" });
    expect(validateSessionText(original).ok).toBe(true);

    const { text, stats } = compactSessionText(original, { maxChars: 20_000 });
    const check = validateSessionText(text);
    expect(check.ok).toBe(true);
    expect(check.issues).toEqual([]);
    expect(stats.omittedUnits).toBeGreaterThan(0);
    expect(check.stats.entries).toBeLessThan(validateSessionText(original).stats.entries);
    expect(check.stats.toolCalls).toBe(check.stats.toolResults);
    const rows = text.trim().split("\n").map((l) => JSON.parse(l));
    expect(rows[0].type).toBe("session");
    expect(rows[1]).toMatchObject({ type: "custom_message", customType: "opencode-import", display: false });
    expect(rows[1].content).toContain("omitted");
    expect(rows[2].parentId).toBe(rows[1].id);
    // First kept message is a user turn, never mid-tool-loop.
    expect(rows[2].message.role).toBe("user");
  });

  it("is a no-op when the session already fits", () => {
    const src = sessionWithTools(2);
    const normalized = normalizeOpenCodeSession(src, FULL_OPTIONS);
    const original = toSessionJsonl(src, normalized, { cwd: "/tmp/p" });
    const { text, stats } = compactSessionText(original, { maxChars: 10_000_000 });
    expect(stats.omittedUnits).toBe(0);
    expect(validateSessionText(text).ok).toBe(true);
  });

  it("detects chain breaks and orphan tool results", () => {
    const src = sessionWithTools(3);
    const normalized = normalizeOpenCodeSession(src, FULL_OPTIONS);
    const rows = toSessionJsonl(src, normalized, { cwd: "/tmp/p" })
      .trim()
      .split("\n")
      .map((l) => JSON.parse(l));
    rows[3].parentId = "bogus";
    const broken = validateSessionText(rows.map((r) => JSON.stringify(r)).join("\n"));
    expect(broken.ok).toBe(false);
    expect(broken.issues.some((i) => i.includes("chain break"))).toBe(true);

    const noResult = rows.filter((r) => r.message?.role !== "toolResult");
    const orphan = validateSessionText(noResult.map((r) => JSON.stringify(r)).join("\n"));
    expect(orphan.ok).toBe(false);
    expect(orphan.issues.some((i) => i.includes("no toolResult"))).toBe(true);
  });
});
