// task-state — external task list that survives compaction (Claude Code
// TodoWrite pattern, REPORT-02 step 9 / PLAN.md R2.4).
//
// Mid-run compaction eats conversation history and the model drifts from the
// original decomposition — the most expensive failure on an 11 t/s model
// (post-compaction thrash re-does finished work). The fix: the task list is
// NOT conversation, it is external state — persisted by a tool call and
// re-injected from disk every turn as a compact tail message.
//
// Pure module: parse/validate/render — no I/O, no pi imports.

/** One todo item. Status vocabulary matches R2.4 (open/doing/done/killed). */
export interface TaskItem {
  id: number;
  description: string;
  status: "pending" | "in_progress" | "completed" | "cancelled";
}

/** Extension config. Kill switch per Golden Rule 15. */
export interface TaskStateConfig {
  enabled: boolean;
  /** R2.4: 15-item / ~200-token cap; extras hard-truncated with [+N more]. */
  maxItems: number;
  /** Directory holding <session>.json task files (gateway reader: planned, see index.ts note). */
  dir: string;
}

export function taskStateConfig(env: NodeJS.ProcessEnv = process.env): TaskStateConfig {
  return {
    enabled: env.TRIS_NO_TASK_STATE !== "1",
    maxItems: 15,
    dir: env.TRIS_TASKS_DIR || ".pi/tasks",
  };
}

const STATUSES = new Set(["pending", "in_progress", "completed", "cancelled"]);

/**
 * Validate raw tool input into a clean list. Returns an error string instead
 * of throwing — the tool result tells the model what to fix.
 */
export function validateTasks(input: unknown): { tasks?: TaskItem[]; error?: string } {
  if (!Array.isArray(input)) return { error: "tasks must be an array" };
  if (input.length > 40) return { error: "too many tasks (max 40 in file; 15 are injected)" };
  const out: TaskItem[] = [];
  for (let i = 0; i < input.length; i++) {
    const raw = input[i] as Record<string, unknown>;
    if (typeof raw !== "object" || raw === null) return { error: `task ${i}: not an object` };
    const description = String(raw.description ?? "").trim();
    if (!description) return { error: `task ${i}: description required` };
    const rawStatus = String(raw.status ?? "pending");
    if (!STATUSES.has(rawStatus)) return { error: `task ${i}: status "${rawStatus}" not in pending|in_progress|completed|cancelled` };
    const status = rawStatus as TaskItem["status"];
    const id = Number.isFinite(Number(raw.id)) ? Math.floor(Number(raw.id)) : i + 1;
    out.push({ id, description: description.slice(0, 200), status });
  }
  return { tasks: out };
}

const MARKS: Record<TaskItem["status"], string> = {
  pending: "[ ]",
  in_progress: "[>]",
  completed: "[x]",
  cancelled: "[-]",
};

/**
 * Render the injected tail block: compact checklist capped at maxItems.
 * Empty list renders "" (injection skipped by injectionResult).
 */
export function renderTaskBlock(tasks: TaskItem[], maxItems = 15): string {
  if (tasks.length === 0) return "";
  const shown = tasks.slice(0, maxItems);
  const lines = shown.map((t) => `${MARKS[t.status]} ${t.id}. ${t.description}`);
  if (tasks.length > maxItems) lines.push(`[+${tasks.length - maxItems} more]`);
  const open = tasks.filter((t) => t.status !== "completed" && t.status !== "cancelled").length;
  const header = `## Task state (external truth — survives compaction; update with update_tasks)`;
  const footer = open === 0 ? `All ${tasks.length} task(s) resolved.` : `${tasks.length - open}/${tasks.length} done, ${open} open.`;
  return `\n\n${header}\n${lines.join("\n")}\n${footer}`;
}
