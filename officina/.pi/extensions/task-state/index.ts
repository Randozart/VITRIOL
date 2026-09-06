import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { requestSidebarUpdate } from "../_shared/sidebar.ts";
import { renderTaskBlock, taskStateConfig, validateTasks, type TaskItem } from "./state.ts";

// task-state — external task list (R2.4 / REPORT-02 step 9, Claude Code
// TodoWrite pattern). The list lives in .officina/tasks/<session>.json (written
// by a tool call, never by conversation) and is re-injected as a tail message
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

// Session-scoped task file path (2026-09-06): globalThis-backed singleton —
// jiti (moduleCache: false) gives each import site its own module instance,
// and session-panel imports getTaskSummary/getTaskItems from its own copy.
// Without sharing, that copy's path stays the module-init default forever
// (stale project file / empty task rows) no matter what session_start does
// on the -e-loaded instance.
const sessionState: { file: string } =
  (globalThis as any).__officinaTaskFileState ??
  ((globalThis as any).__officinaTaskFileState = { file: join(taskStateConfig().dir, "default.json") });

/** Read a task file; missing/corrupt = empty. */
function readTasksFile(file: string): TaskItem[] {
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8")) as { tasks?: unknown };
    return validateTasks(parsed.tasks ?? []).tasks ?? [];
  } catch {
    return [];
  }
}

/** Read the current session's task file. No cross-session fallback — each
 *  session owns its own list. The legacy .pi/tasks/ bridge outlived its
 *  purpose (writes have gone to .officina/tasks/ since 2026-09-02). */
function readTasksAny(): TaskItem[] {
  return readTasksFile(sessionState.file);
}

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
  const tasks = readTasksAny();
  if (tasks.length === 0) return null;
  return {
    total: tasks.length,
    pending: tasks.filter((t) => t.status === "pending").length,
    inProgress: tasks.filter((t) => t.status === "in_progress").length,
    completed: tasks.filter((t) => t.status === "completed").length,
    cancelled: tasks.filter((t) => t.status === "cancelled").length,
  };
}

/** Raw task items (content, not just counts) for sidebar display.
 *  Open items first (in_progress, then pending), then the rest. */
export function getTaskItems(): TaskItem[] {
  return readTasksAny().sort((a, b) => rankOf(a.status) - rankOf(b.status));
}

function rankOf(s: TaskItem["status"]): number {
  return s === "in_progress" ? 0 : s === "pending" ? 1 : 2;
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
    const stem = sessionFileStem(sm?.getSessionFile?.());
    sessionState.file = join(cfg.dir, `${stem}.json`);
    // Sidebar re-render AFTER the module var is correct (2026-09-06): pi fires
    // session_start in extension load order (scratchpad → session-panel →
    // task-state), and session-panel's own session_start renders the sidebar
    // BEFORE this handler runs — reading the stale path. This late update
    // heals the race; /new then shows the (empty) new session's state.
    requestSidebarUpdate();
  });

  // session_shutdown (2026-09-06): /new replaces the session IN-PROCESS — the
  // extension instances and their module-level state survive. Reset the path
  // to the default so nothing from the old session can be read or written
  // between teardown and the next session_start (pi fires shutdown on
  // teardown; previously nothing handled it, so sessionState.file carried the
  // old session's stem through the swap).
  pi.on("session_shutdown", async () => {
    sessionState.file = join(cfg.dir, "default.json");
    requestSidebarUpdate();
  });

  function readTasks(): TaskItem[] {
    return readTasksAny();
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
        mkdirSync(dirname(sessionState.file), { recursive: true });
        writeFileSync(sessionState.file, JSON.stringify({ updated: Date.now(), tasks: v.tasks }, null, 2));
      } catch (e) {
        return { content: [{ type: "text" as const, text: `update_tasks could not persist: ${(e as Error).message}` }], details: {}, isError: true };
      }
      const done = (v.tasks ?? []).filter((t) => t.status === "completed").length;
      emitHarnessEvent(harnessEvent("lc-tasks", "updated", {
        detail: `${done}/${(v.tasks ?? []).length} done`,
        session: sessionState.file.split("/").pop()?.replace(/\.json$/, ""),
      }));
      // Live sidebar refresh (2026-09-04): the harness event above is a passive
      // log; the sidebar's task section only re-renders when asked. Mutators of
      // sidebar-visible state request the update after persisting.
      requestSidebarUpdate();
      return { content: [{ type: "text" as const, text: `task state saved: ${done}/${(v.tasks ?? []).length} done → ${sessionState.file}` }], details: {} };
    },
  });

  pi.on("context", async (event) => {
    // Guard: session_start hasn't fired yet → sessionState.file is still the
    // module-init default. Don't inject stale tasks from a previous session's
    // default.json — return nothing so the model starts with a clean slate.
    if (sessionState.file.endsWith("/default.json")) return undefined;
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
