import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Live sidebar-refresh contract (2026-09-04): mutators of sidebar-visible
// state (task list, scratchpad) call requestSidebarUpdate() after persisting.
// Regression: update_tasks / scratchpad_write previously only appended a
// passive JSONL event, so the sidebar stayed stale until an unrelated
// refresh (engine poll / message_end).
//
// 2026-09-04 fix: globalThis-backed singletons in _shared/sidebar.ts ensure
// the listener registered by session-panel is visible to task-state/scratchpad
// even when jiti loads each extension in a separate module scope. The old mock
// hid this bug — these tests now verify the REAL cross-module notification.

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
  let unsub: (() => void) | undefined;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "taskstate-sidebar-"));
    process.env.TRIS_TASKS_DIR = dir;
  });
  afterEach(() => {
    unsub?.();
    unsub = undefined;
    delete process.env.TRIS_TASKS_DIR;
    rmSync(dir, { recursive: true, force: true });
  });

  it("update_tasks fires the real sidebar listener after a successful persist", async () => {
    const { onSidebarUpdate } = await import("../_shared/sidebar.ts");
    const mod = await import("./index.ts");

    let fired = 0;
    unsub = onSidebarUpdate(() => { fired++; });

    const { pi, tools } = fakePi();
    mod.default(pi as never);
    const out = await tools["update_tasks"].execute("1", {
      tasks: [{ id: 1, description: "first task", status: "in_progress" }],
    });
    expect(out.isError).toBeFalsy();
    expect(String(out.content?.[0]?.text ?? "")).toContain("task state saved");
    expect(fired).toBe(1);
  });

  it("update_tasks does NOT fire the sidebar listener when validation fails", async () => {
    const { onSidebarUpdate } = await import("../_shared/sidebar.ts");
    const mod = await import("./index.ts");

    let fired = 0;
    unsub = onSidebarUpdate(() => { fired++; });

    const { pi, tools } = fakePi();
    mod.default(pi as never);
    const out = await tools["update_tasks"].execute("1", {
      tasks: [{ status: "pending" }], // missing description → rejected
    });
    expect(out.isError).toBe(true);
    expect(fired).toBe(0);
  });
});

describe("scratchpad → sidebar refresh contract", () => {
  let dir: string;
  let unsub: (() => void) | undefined;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "scratch-sidebar-"));
    process.env.OFFICINA_SCRATCHPAD_DIR = dir;
  });
  afterEach(() => {
    unsub?.();
    unsub = undefined;
    delete process.env.OFFICINA_SCRATCHPAD_DIR;
    rmSync(dir, { recursive: true, force: true });
  });

  it("scratchpad_write fires the real sidebar listener after a successful persist", async () => {
    const { onSidebarUpdate } = await import("../_shared/sidebar.ts");
    const mod = await import("../scratchpad/index.ts");

    let fired = 0;
    unsub = onSidebarUpdate(() => { fired++; });

    const { pi, tools } = fakePi();
    mod.default(pi as never);
    const out = await tools["scratchpad_write"].execute("1", {
      facts: ["the engine was OOM-killed at 15:34"],
    });
    expect(out.isError).toBeFalsy();
    expect(String(out.content?.[0]?.text ?? "")).toContain("scratchpad saved");
    expect(fired).toBe(1);
  });
});
