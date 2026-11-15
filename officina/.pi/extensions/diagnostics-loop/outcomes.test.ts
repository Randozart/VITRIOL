import { describe, it, expect, vi, beforeEach } from "vitest";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const mocks = vi.hoisted(() => ({ execute: vi.fn(), record: vi.fn() }));
vi.mock("node:child_process", () => ({ execFileSync: mocks.execute }));
vi.mock("../_shared/toolchecks.ts", () => ({ recordToolCheck: mocks.record }));
import install from "./index.ts";

beforeEach(() => { vi.clearAllMocks(); mocks.execute.mockReset(); });

describe("checker outage reporting", () => {
  it("records unavailable facts, warns once per outage and never injects false repairs", async () => {
    const handlers = new Map<string, (...args: any[]) => any>();
    install({ on: (name: string, handler: (...args: any[]) => any) => handlers.set(name, handler) } as unknown as ExtensionAPI);
    const notify = vi.fn();
    const tool = handlers.get("tool_result")!;
    const context = handlers.get("context")!;
    const event = { toolName: "edit", toolCallId: "test-call", input: { path: "a.py" } };
    mocks.execute.mockImplementation(() => { throw Object.assign(new Error("denied"), { code: "EPERM", status: 0 }); });
    await tool(event, { ui: { notify } });
    await tool(event, { ui: { notify } });
    expect(notify).toHaveBeenCalledTimes(1);
    expect(mocks.record).toHaveBeenLastCalledWith("test-call", { syntax: { ok: false, available: false, output: expect.stringContaining("EPERM") } });
    expect(await context({ messages: [] })).toBeUndefined();
    mocks.execute.mockReturnValue("");
    await tool(event, { ui: { notify } });
    expect(mocks.record).toHaveBeenLastCalledWith("test-call", { syntax: { ok: true, available: true, output: undefined } });
    mocks.execute.mockImplementation(() => { throw Object.assign(new Error("invalid code"), { status: 1, stderr: "SyntaxError: broken" }); });
    await tool(event, { ui: { notify } });
    const result = await context({ messages: [] });
    expect(result.messages[0].content).toContain("fix before continuing");
    expect(result.messages[0].content).toContain("SyntaxError");
    expect(await context({ messages: [] })).toBeUndefined();
    mocks.execute.mockImplementation(() => { throw Object.assign(new Error("denied"), { code: "EPERM" }); });
    await tool(event, { ui: { notify } });
    expect(notify).toHaveBeenCalledTimes(2);
  });
});
