import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Live sidebar-refresh contract (2026-09-04): mutators of sidebar-visible
// state (task list, scratchpad) call requestSidebarUpdate() after persisting.
// Regression: update_tasks / scratchpad_write previously only appended a
// passive JSONL event, so the sidebar stayed stale until an unrelated
// refresh (engine poll / message_end). Spied via module mock; config dirs
// are pointed at tmp dirs through env BEFORE the modules load (dynamic
// import), so the tests never touch a real project's state.

vi.mock("../_shared/sidebar.ts", () => ({ requestSidebarUpdate: vi.fn() }));

interface ToolLike {
  execute(id: string, input: unknown): Promise<{ content?: Array<{ text?: string }>; isError?: boolean }>;
}

function fakePi(): { pi: Record<string, unknown>; tools: Record<string, ToolLike> } {
  const tools: Record<string, ToolLike> = {};
  const pi: Record<string, unknown> = {
    registerTool: (t: { name: string; execute: ToolLike["execute"] }) => {
      tools[t.name] = { execute: t.execute };
    },
    on: () => {},
    registerCommand: () => {},
  };
  return { pi, tools };
}

describe("task-state → sidebar refresh contract", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "taskstate-sidebar-"));
    process.env.TRIS_TASKS_DIR = dir;
  });
  afterEach(() => {
    delete process.env.TRIS_TASKS_DIR;
    rmSync(dir, { recursive: true, force: true });
  });

  it("update_tasks requests a sidebar update after a successful persist", async () => {
    const mod = await import("./index.ts");
    const { requestSidebarUpdate } = await import("../_shared/sidebar.ts");
    const mocked = vi.mocked(requestSidebarUpdate);
    mocked.mockClear();
    const { pi, tools } = fakePi();
    mod.default(pi as never);
    const out = await tools["update_tasks"].execute("1", {
      tasks: [{ id: 1, description: "first task", status: "in_progress" }],
    });
    expect(out.isError).toBeFalsy();
    expect(String(out.content?.[0]?.text ?? "")).toContain("task state saved");
    expect(mocked).toHaveBeenCalledTimes(1);
  });

  it("update_tasks does NOT request a sidebar update when validation fails", async () => {
    const mod = await import("./index.ts");
    const { requestSidebarUpdate } = await import("../_shared/sidebar.ts");
    const mocked = vi.mocked(requestSidebarUpdate);
    mocked.mockClear();
    const { pi, tools } = fakePi();
    mod.default(pi as never);
    const out = await tools["update_tasks"].execute("1", {
      tasks: [{ status: "pending" }], // missing description → rejected
    });
    expect(out.isError).toBe(true);
    expect(mocked).not.toHaveBeenCalled();
  });
});

describe("scratchpad → sidebar refresh contract", () => {
  let dir: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "scratch-sidebar-"));
    process.env.OFFICINA_SCRATCHPAD_DIR = dir;
  });
  afterEach(() => {
    delete process.env.OFFICINA_SCRATCHPAD_DIR;
    rmSync(dir, { recursive: true, force: true });
  });

  it("scratchpad_write requests a sidebar update after a successful persist", async () => {
    const mod = await import("../scratchpad/index.ts");
    const { requestSidebarUpdate } = await import("../_shared/sidebar.ts");
    const mocked = vi.mocked(requestSidebarUpdate);
    mocked.mockClear();
    const { pi, tools } = fakePi();
    mod.default(pi as never);
    const out = await tools["scratchpad_write"].execute("1", {
      facts: ["the engine was OOM-killed at 15:34"],
    });
    expect(out.isError).toBeFalsy();
    expect(String(out.content?.[0]?.text ?? "")).toContain("scratchpad saved");
    expect(mocked).toHaveBeenCalledTimes(1);
  });
});
