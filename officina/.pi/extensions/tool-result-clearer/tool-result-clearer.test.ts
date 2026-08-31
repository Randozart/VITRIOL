import { describe, it, expect } from "vitest";
import {
  clearConfig,
  isExcluded,
  DEFAULT_EXCLUDE,
  estimateTokens,
  stubFor,
  planClear,
  type ToolResultLike,
} from "./index.ts";

const bigOutput = "x".repeat(4000); // ~1000 tokens by the chars/4 estimate

function toolResult(over: Partial<ToolResultLike>): ToolResultLike {
  return {
    role: "toolResult",
    toolCallId: "call_1",
    toolName: "bash",
    content: [{ type: "text", text: bigOutput }],
    details: {},
    isError: false,
    timestamp: Date.now(),
    ...over,
  };
}

describe("clearConfig", () => {
  it("defaults to enabled with keep=4 and the config-listed excludes", () => {
    const cfg = clearConfig({});
    expect(cfg.enabled).toBe(true);
    expect(cfg.keep).toBe(4);
    for (const t of DEFAULT_EXCLUDE) expect(cfg.exclude).toContain(t);
  });

  it("LITTLE_CODER_CLEAR_EXCLUDE extends, dedupes, case-insensitively", () => {
    const cfg = clearConfig({ LITTLE_CODER_CLEAR_EXCLUDE: "My_Tool, read_plan ," });
    expect(cfg.exclude).toContain("my_tool");
    expect(cfg.exclude.filter((x) => x === "read_plan")).toHaveLength(1);
  });

  it("honors a keep override", () => {
    expect(clearConfig({ LITTLE_CODER_CLEAR_KEEP: "2" }).keep).toBe(2);
  });

  it("falls back to 4 for non-numeric or <1 keep", () => {
    expect(clearConfig({ LITTLE_CODER_CLEAR_KEEP: "lots" }).keep).toBe(4);
    expect(clearConfig({ LITTLE_CODER_CLEAR_KEEP: "0" }).keep).toBe(4);
  });

  it("hard-off via the kill switch", () => {
    expect(clearConfig({ LITTLE_CODER_NO_CLEAR_TOOL_RESULTS: "1" }).enabled).toBe(false);
  });
});

describe("estimateTokens", () => {
  it("estimates chars/4 and skips non-text content", () => {
    expect(estimateTokens([{ type: "text", text: "abcd" }])).toBe(1);
    expect(
      estimateTokens([{ type: "text", text: "abcd" }, { type: "image", image: "data:image/png;base64,AAAA" } as never]),
    ).toBe(1);
  });
});

describe("stubFor", () => {
  it("names the tool and the freed token count", () => {
    const stub = stubFor(toolResult({ toolName: "read" }), 1000);
    expect(stub).toContain("read");
    expect(stub).toContain("~1000 tokens");
  });
});

describe("planClear", () => {
  const user = { role: "user", content: "hi" };

  it("returns messages unchanged when disabled", () => {
    const messages = [toolResult({}), toolResult({})];
    const plan = planClear(messages, { enabled: false, keep: 4, exclude: [] });
    expect(plan.cleared).toBe(0);
    expect(plan.messages).toBe(messages);
  });

  it("clears nothing when there are <= keep results", () => {
    const messages = [user, toolResult({}), toolResult({})];
    const plan = planClear(messages, { enabled: true, keep: 4, exclude: [] });
    expect(plan.cleared).toBe(0);
    expect(plan.messages).toBe(messages);
  });

  it("stubs older results but keeps the last N verbatim", () => {
    const r1 = toolResult({ toolCallId: "call_1" });
    const r2 = toolResult({ toolCallId: "call_2" });
    const r3 = toolResult({ toolCallId: "call_3" });
    const messages = [user, r1, r2, r3];
    const plan = planClear(messages, { enabled: true, keep: 2, exclude: [] });
    expect(plan.cleared).toBe(1);
    expect(plan.freedTokens).toBe(1000);

    const stubbed = plan.messages[1] as ToolResultLike;
    expect(stubbed.content[0].type).toBe("text");
    const stubText = stubbed.content[0].type === "text" ? stubbed.content[0].text : "";
    expect(stubText).toContain("call_1");
    // The two newest stay intact.
    expect((plan.messages[2] as ToolResultLike).toolCallId).toBe("call_2");
    expect((plan.messages[3] as ToolResultLike).toolCallId).toBe("call_3");
  });

  it("never clears error results", () => {
    const err = toolResult({ toolCallId: "call_1", isError: true });
    const ok1 = toolResult({ toolCallId: "call_2" });
    const ok2 = toolResult({ toolCallId: "call_3" });
    const messages = [err, ok1, ok2];
    const plan = planClear(messages, { enabled: true, keep: 1, exclude: [] });
    // Only one non-error result can be cleared (the err is exempted).
    expect(plan.cleared).toBe(1);
    expect((plan.messages[0] as ToolResultLike).isError).toBe(true);
  });

  it("never stubs excluded tools (state survives regardless of keep window)", () => {
    const plan = toolResult({ toolName: "read_plan", toolCallId: "p1" });
    const bash1 = toolResult({ toolName: "bash", toolCallId: "b1" });
    const bash2 = toolResult({ toolName: "bash", toolCallId: "b2" });
    const msgs = [plan, bash1, bash2];
    const out = planClear(msgs, { enabled: true, keep: 1, exclude: DEFAULT_EXCLUDE.map((x) => x.toLowerCase()) });
    expect(out.cleared).toBe(1); // only the oldest NON-excluded result
    expect((out.messages[0] as ToolResultLike).content[0]).toEqual({ type: "text", text: "x".repeat(4000) }); // plan intact
  });

  it("isExcluded matches case-insensitively on toolName", () => {
    const cfg = clearConfig({});
    expect(isExcluded(toolResult({ toolName: "Update_Tasks" }), cfg)).toBe(true);
    expect(isExcluded(toolResult({ toolName: "bash" }), cfg)).toBe(false);
    expect(isExcluded({ role: "user" }, cfg)).toBe(false);
  });

  it("preserves message order and non-tool messages", () => {
    const r1 = toolResult({ toolCallId: "call_1" });
    const r2 = toolResult({ toolCallId: "call_2" });
    const messages = [user, r1, user, r2];
    const plan = planClear(messages, { enabled: true, keep: 1, exclude: [] });
    expect(plan.cleared).toBe(1);
    expect((plan.messages[1] as ToolResultLike).toolCallId).toBe("call_1"); // stubbed in place
    expect(plan.messages[0]).toBe(user);
    expect(plan.messages[2]).toBe(user);
    expect((plan.messages[3] as ToolResultLike).toolCallId).toBe("call_2");
  });
});