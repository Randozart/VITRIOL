import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { renderTaskBlock, taskStateConfig, validateTasks, type TaskItem } from "./state.ts";

// task-state — external task list (R2.4 / REPORT-02 step 9, Claude Code
// TodoWrite pattern). The list lives in .pi/tasks/<session>.json (written by
// a tool call, never by conversation) and is re-injected as a tail message
// before EVERY LLM call — so it survives mid-run compaction by construction:
// it is re-read from disk, not from history.
//
// Cache safety (Rule 7): we only ever APPEND to event.messages — the existing
// prefix stays byte-identical, KV cache intact. A copy is skipped when the
// last injected copy already carries the identical block (no pile-up on
// unchanged state); when the list CHANGES we inject the new tail, leaving the
// stale copy in history — never mutate or delete what is already cached.
//
// Hermes-facing note (2026-08-29 audit): the gateway does NOT read this
// directory yet — cross-session task visibility is PLANNED (queue:
// POST-MIGRATION-PLAN.md "gateway task-view"), never silently assumed.
// Kill switch: TRIS_NO_TASK_STATE=1.

const CUSTOM_TYPE = "lc-tasks";

// Module-level task file path — updated on session_start, read by getTaskSummary.
let currentTaskFile = join(taskStateConfig().dir, "default.json");

// ── Sidebar data export ──────────────────────────────────────────────────
export interface TaskSummary {
  total: number;
  pending: number;
  inProgress: number;
  completed: number;
  cancelled: number;
}

/** Read the current task file and return a summary. Returns null if no tasks. */
export function getTaskSummary(): TaskSummary | null {
  try {
    const parsed = JSON.parse(readFileSync(currentTaskFile, "utf8")) as { tasks?: unknown };
    const v = validateTasks(parsed.tasks ?? []);
    const tasks = v.tasks ?? [];
    if (tasks.length === 0) return null;
    return {
      total: tasks.length,
      pending: tasks.filter((t) => t.status === "pending").length,
      inProgress: tasks.filter((t) => t.status === "in_progress").length,
      completed: tasks.filter((t) => t.status === "completed").length,
      cancelled: tasks.filter((t) => t.status === "cancelled").length,
    };
  } catch {
    return null;
  }
}

/** Session stem used as the task filename (cross-session visibility for Hermes). */
export function sessionFileStem(sessionFile: string | null | undefined): string {
  if (!sessionFile) return "default";
  return sessionFile.split("/").pop()?.replace(/\.jsonl$/, "") ?? "default";
}

export default function (pi: ExtensionAPI) {
  const cfg = taskStateConfig();
  if (!cfg.enabled) return;

  pi.on("session_start", async (_event, ctx) => {
    const sm = (ctx as { sessionManager?: { getSessionFile?: () => string | null } }).sessionManager;
    currentTaskFile = join(cfg.dir, `${sessionFileStem(sm?.getSessionFile?.())}.json`);
  });

  function readTasks(): TaskItem[] {
    try {
      const parsed = JSON.parse(readFileSync(currentTaskFile, "utf8")) as { tasks?: unknown };
      const v = validateTasks(parsed.tasks ?? []);
      return v.tasks ?? [];
    } catch {
      return []; // no file yet / corrupt — inject nothing, keep working
    }
  }

  pi.registerTool({
    name: "update_tasks",
    label: "Update Tasks",
    description:
      "Replace the session task list (external state, survives compaction, re-injected every turn). " +
      "Keep <=15 items. Call it whenever progress changes: mark exactly one item in_progress while working.",
    parameters: Type.Object({
      tasks: Type.Array(
        Type.Object({
          id: Type.Optional(Type.Number()),
          description: Type.String(),
          status: Type.Union([
            Type.Literal("pending"),
            Type.Literal("in_progress"),
            Type.Literal("completed"),
            Type.Literal("cancelled"),
          ]),
        }),
        { description: "Full replacement list, ordered" },
      ),
    }),
    async execute(_id, { tasks }) {
      const v = validateTasks(tasks);
      if (v.error) {
        return { content: [{ type: "text" as const, text: `update_tasks rejected: ${v.error}` }], details: {}, isError: true };
      }
      try {
        mkdirSync(dirname(currentTaskFile), { recursive: true });
        writeFileSync(currentTaskFile, JSON.stringify({ updated: Date.now(), tasks: v.tasks }, null, 2));
      } catch (e) {
        return { content: [{ type: "text" as const, text: `update_tasks could not persist: ${(e as Error).message}` }], details: {}, isError: true };
      }
      const done = (v.tasks ?? []).filter((t) => t.status === "completed").length;
      emitHarnessEvent(harnessEvent("lc-tasks", "updated", { detail: `${done}/${(v.tasks ?? []).length} done` }));
      return { content: [{ type: "text" as const, text: `task state saved: ${done}/${(v.tasks ?? []).length} done → ${currentTaskFile}` }], details: {} };
    },
  });

  pi.on("context", async (event) => {
    const block = renderTaskBlock(readTasks(), cfg.maxItems);
    if (!block) return undefined;
    if (lastCopyIsCurrent(event.messages, block)) return undefined;
    const tail = {
      role: "custom" as const,
      customType: CUSTOM_TYPE,
      content: block,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}

/** True when the most recent injected copy already carries exactly `block`. */
export function lastCopyIsCurrent(messages: unknown[], block: string): boolean {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i] as { role?: string; customType?: string; content?: unknown };
    if (m.role === "custom" && m.customType === CUSTOM_TYPE) {
      return String(m.content ?? "") === block;
    }
  }
  return false; // never injected this session yet
}
