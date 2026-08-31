import { describe, it, expect } from "vitest";
import {
  buildRelayPrompt,
  condenseTranscript,
  generateCard,
  parseCard,
  relayPath,
  renderRelayTail,
  shouldInject,
  RELAY_BUDGET_CHARS,
  type RelayCard,
} from "./relay.ts";

const card = (over: Partial<RelayCard> = {}): RelayCard => ({
  from_model: "A.gguf", to_model: "B.gguf", session: "s", at: 0,
  card: "GOAL: finish ext\nOPEN ERRORS: none", injected: false, ...over,
});

describe("condenseTranscript", () => {
  it("keeps only user/assistant, capped and tail-first", () => {
    const msgs = [
      { role: "user", content: "task one" },
      { role: "toolResult", content: "HUGE".repeat(5000) },
      { role: "assistant", content: [{ type: "text", text: "did it" }] },
    ];
    const out = condenseTranscript(msgs);
    expect(out).toContain("user: task one");
    expect(out).toContain("assistant: did it");
    expect(out).not.toContain("HUGE");
    expect(out.length).toBeLessThan(1000);
  });

  it("tail window respected", () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ role: "user", content: `msg-${i}` }));
    const out = condenseTranscript(many, 12);
    expect(out).toContain("msg-39");
    expect(out).not.toContain("msg-10");
  });
});

describe("parseCard", () => {
  it("rejects structurally wrong output", () => {
    expect(parseCard("sure! here's some prose")).toBeNull();
    expect(parseCard("")).toBeNull();
  });

  it("accepts GOAL-sectioned card and caps budget", () => {
    const ok = parseCard("GOAL: ship relay\nDECISIONS: dark ship");
    expect(ok).toContain("GOAL");
    const fat = parseCard("GOAL: " + "x".repeat(5000));
    expect((fat ?? "").length).toBeLessThan(RELAY_BUDGET_CHARS + 40);
  });
});

describe("shouldInject", () => {
  it("only for the incoming model, only once, only armed", () => {
    expect(shouldInject(card(), "B.gguf", true)).toBe(true);
    expect(shouldInject(card({ injected: true }), "B.gguf", true)).toBe(false);
    expect(shouldInject(card(), "A.gguf", true)).toBe(false);
    expect(shouldInject(card(), "B.gguf", false)).toBe(false);
    expect(shouldInject(card(), "", true)).toBe(false);
  });
});

describe("generateCard (mock fetch — no engine needed)", () => {
  const okFetch = (content: string) =>
    (async () => ({ ok: true, json: async () => ({ choices: [{ message: { content } }] }) })) as unknown as typeof fetch;

  it("posts to /v1/chat/completions with the outgoing model", async () => {
    let seenUrl = "";
    let seenBody: Record<string, unknown> = {};
    const spy = (async (u: unknown, init?: RequestInit) => {
      seenUrl = String(u);
      seenBody = JSON.parse(String(init?.body));
      return { ok: true, json: async () => ({ choices: [{ message: { content: "GOAL: relay works" } }] }) };
    }) as unknown as typeof fetch;
    const out = await generateCard("http://x:8279", "A.gguf", "prompt", spy);
    expect(seenUrl).toBe("http://x:8279/v1/chat/completions");
    expect(seenBody.model).toBe("A.gguf");
    expect(out).toBe("GOAL: relay works");
  });

  it("falls back to reasoning_content when content is empty (thinking models)", async () => {
    const f = (async () => ({ ok: true, json: async () => ({ choices: [{ message: { content: "", reasoning_content: "GOAL: via reasoning" } }] }) })) as unknown as typeof fetch;
    expect(await generateCard("http://x", "M", "p", f)).toBe("GOAL: via reasoning");
  });

  it("network failure yields null, never throws", async () => {
    const boom = (async () => {
      throw new Error("ECONNREFUSED");
    }) as unknown as typeof fetch;
    expect(await generateCard("http://x", "M", "p", boom)).toBeNull();
  });

  it("HTTP error yields null", async () => {
    const f = (async () => ({ ok: false, json: async () => ({}) })) as unknown as typeof fetch;
    expect(await generateCard("http://x", "M", "p", f)).toBeNull();
  });
});

describe("relayPath + renderRelayTail", () => {
  it("session keys are traversal-safe", () => {
    const p = relayPath(".pi/relay", "../evil");
    expect(p.startsWith(".pi/relay/")).toBe(true);
    expect(p.split("/")).toHaveLength(3);
    expect(relayPath(".pi/relay", "")).toBe(".pi/relay/default.json");
  });

  it("tail names the outgoing model and the budget note", () => {
    const t = renderRelayTail(card());
    expect(t).toContain("from A.gguf");
    expect(t).toContain("GOAL: finish ext");
    expect(t).toContain("~500-token");
  });
});

describe("buildRelayPrompt", () => {
  it("demands the five-section contract and includes transcript", () => {
    const p = buildRelayPrompt("user: do the thing");
    expect(p).toContain("GOAL:");
    expect(p).toContain("OPEN ERRORS:");
    expect(p).toContain("user: do the thing");
  });
});
