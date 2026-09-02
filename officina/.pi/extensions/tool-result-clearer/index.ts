import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { isActive, tickTurn, registerTaskFiles } from "../_shared/active-files.ts";

/** A content block in a tool result message (text or image). */
export type ContentBlock =
  | { type: "text"; text: string }
  | { type: "image"; image: string };

/** Structural shape of a tool-result message as it appears in the message list. */
export interface ToolResultLike {
  role: "toolResult";
  toolCallId: string;
  toolName?: string;
  content: ContentBlock[];
  details?: unknown;
  isError: boolean;
  timestamp?: number;
}

// Tool-result clearing (R2.1, Claude Code "context editing" — clear_tool_uses).
//
// In agentic loops the single biggest context waste is stale tool results: a
// file read at turn 3, a test-run output at turn 5, a directory listing at
// turn 7 — all still sitting in context at turn 25, all already reflected in
// the model's subsequent edits. Standard compaction treats them as normal
// conversation; clearing removes them at the source.
//
// pi exposes the right hook: the `context` event fires before EVERY LLM call
// with the full message list, and returning `{ messages }` REPLACES what gets
// sent to the provider (runner.js emitContext). We walk the list, keep the
// last N tool results verbatim, and replace older ones with a compact stub
// that preserves the causal record (tool name + target + token count freed)
// without the payload. The session file still holds the full results — this
// only trims what the model re-reads each turn.
//
// Why this runs BEFORE compaction: clearing is free (no LLM summarization),
// and it is exactly where Claude Code measured an 84% token reduction with
// the memory tool. It complements little-coder's read-guard (prevents large
// results entering) and rtk-output (compresses output at ingestion) by
// evicting results after they have been consumed.
//
// Active-file protection: tool results for files that have been recently
// edited or read (tracked by _shared/active-files.ts) are kept verbatim
// even if they fall outside the keep window. This prevents the model from
// losing context about files it is actively working on.
//
// Tuning / opt-out:
//   LITTLE_CODER_NO_CLEAR_TOOL_RESULTS=1   hard off (kill switch, Rule 15)
//   LITTLE_CODER_CLEAR_KEEP=<n>            tool results kept verbatim (default 12)
//   LITTLE_CODER_CLEAR_EXCLUDE=a,b         extra tool names never cleared
//                                          (merged onto DEFAULT_EXCLUDE)
//   OFFICINA_NO_ACTIVE_FILES=1             disable active-file protection
//   OFFICINA_ACTIVE_TTL=<n>                active-file TTL in turns (default 10)

/** Tools whose results are load-bearing state, never consumable output.
 *  Mirrors ~/.config/trismegistus/config.yaml context_pipeline.clear.
 *  exclude_tools — implementer side of that promise (2026-08-29: the config
 *  listed exclusions this extension did not yet honor; now it does). */
export const DEFAULT_EXCLUDE = ["read_plan", "todo_state", "vitriol_status", "update_tasks"];

export interface ClearConfig {
  enabled: boolean;
  keep: number;
  exclude: string[]; // lowercased tool names never cleared
}

export function clearConfig(env: NodeJS.ProcessEnv = process.env): ClearConfig {
  const keepRaw = env.LITTLE_CODER_CLEAR_KEEP;
  const keep = keepRaw !== undefined && keepRaw.trim() !== "" ? Number(keepRaw) : 12;
  const extra = (env.LITTLE_CODER_CLEAR_EXCLUDE ?? "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter((s) => s.length > 0);
  return {
    enabled: env.LITTLE_CODER_NO_CLEAR_TOOL_RESULTS !== "1",
    keep: Number.isFinite(keep) && keep >= 1 ? Math.floor(keep) : 12,
    exclude: [...new Set([...DEFAULT_EXCLUDE.map((s) => s.toLowerCase()), ...extra])],
  };
}

/** Rough token estimate — chars / 4. Only used for the freed-token count in the stub. */
export function estimateTokens(content: ContentBlock[]): number {
  let chars = 0;
  for (const c of content) {
    if (c.type === "text") chars += c.text.length;
  }
  return Math.ceil(chars / 4);
}

export function isToolResult(m: unknown): m is ToolResultLike {
  return typeof m === "object" && m !== null && (m as { role?: string }).role === "toolResult";
}

/** True when a tool result's name is on the never-clear exclusion list. */
export function isExcluded(m: unknown, config: ClearConfig): boolean {
  if (!isToolResult(m) || !m.toolName) return false;
  return config.exclude.includes(m.toolName.toLowerCase());
}

/**
 * Build the stub that replaces a consumed tool result.
 * Preserves tool identity and a one-line causal record; drops the payload.
 */
export function stubFor(original: ToolResultLike, freedTokens: number): string {
  const name = original.toolName ?? "tool";
  return `[tool result cleared: ~${freedTokens} tokens — ${name} (${original.toolCallId}; result consumed; full output retained in session file)]`;
}

/** Extract the first text content from a tool result. */
function contentText(content: ContentBlock[]): string {
  const parts: string[] = [];
  for (const c of content) {
    if (c.type === "text") parts.push(c.text);
  }
  return parts.join("\n");
}

/**
 * Summarize a tool result before clearing it. Extracts key facts from file
 * reads and grep results so the model retains knowledge after compaction.
 * Returns null when summarization is not beneficial.
 */
export function summarizeResult(original: ToolResultLike): string | null {
  const name = (original.toolName ?? "").toLowerCase();
  const text = contentText(original.content ?? []);
  if (!text) return null;

  if (name === "read" || name === "read_file") {
    const lines = text.split("\n").filter((l) => l.trim());
    const errors = lines.filter((l) => /error|warning|fail|panic|E\d{4}/i.test(l));
    const head = lines.slice(0, 5);
    const parts = [...new Set([...head, ...errors])].slice(0, 8);
    return parts.length > 0 ? `[from ${original.toolName}]: ${parts.join("; ")}` : null;
  }

  if (name === "grep" || name === "rg") {
    const lines = text.split("\n").filter((l) => l.trim()).slice(0, 10);
    return lines.length > 0 ? `[from ${original.toolName}]: ${lines.join("; ")}` : null;
  }

  return null;
}

/** The result of planning a clear pass over a message list. */
export interface ClearPlan {
  /** Messages to send to the model (a new array when anything changed). */
  messages: unknown[];
  /** Number of tool results replaced with stubs. */
  cleared: number;
  /** Estimated tokens freed this pass. */
  freedTokens: number;
  /** Original results that were cleared (for summary injection). */
  clearedResults: ToolResultLike[];
}

/**
 * Plan which tool results to clear, given the config. Pure — unit-testable.
 *
 * Keeps the last `keep` tool results verbatim (the model still needs recent
 * results to reason about), replaces everything older with a stub, and never
 * touches non-tool messages or the message order. Never clears error results
 * (errors are load-bearing for the loop-breaker) — they stay verbatim.
 *
 * Results for active files (recently edited/read) are kept verbatim even if
 * they fall outside the keep window.
 */
export function planClear(
  messages: unknown[],
  config: ClearConfig,
  isFileActive?: (file: string) => boolean,
  resolveFile?: (m: unknown) => string,
): ClearPlan {
  if (!config.enabled) return { messages, cleared: 0, freedTokens: 0, clearedResults: [] };

  // Collect the tool-result indices in order; keep the last `keep` of them.
  // Excluded tools (plan/todo/state) are never candidates — not cleared and
  // not counted toward the keep window, so protection is absolute.
  const resultIndices: number[] = [];
  for (let i = 0; i < messages.length; i++) {
    if (isToolResult(messages[i]) && !isExcluded(messages[i], config)) resultIndices.push(i);
  }
  const clearThreshold = resultIndices.length - config.keep;
  if (clearThreshold <= 0) return { messages, cleared: 0, freedTokens: 0, clearedResults: [] };

  const out = messages.slice();
  let cleared = 0;
  let freedTokens = 0;
  const clearedResults: ToolResultLike[] = [];
  for (let k = 0; k < clearThreshold; k++) {
    const idx = resultIndices[k];
    const original = out[idx] as ToolResultLike;
    if (original.isError) continue; // errors are never cleared

    // Active-file protection: skip clearing results for actively-edited files
    if (isFileActive && resolveFile) {
      const file = resolveFile(original);
      if (file && isFileActive(file)) continue;
    }

    const tokens = estimateTokens(original.content ?? []);
    clearedResults.push(original);
    out[idx] = {
      ...original,
      content: [{ type: "text", text: stubFor(original, tokens) }],
    };
    cleared += 1;
    freedTokens += tokens;
  }
  return { messages: out, cleared, freedTokens, clearedResults };
}

/** Max number of cleared results to summarize per turn. */
const SUMMARY_MAX = (() => {
  const raw = process.env.OFFICINA_CLEAR_SUMMARY_MAX;
  if (raw === undefined) return 5;
  const n = Number(raw);
  return Number.isFinite(n) && n >= 1 ? Math.floor(n) : 5;
})();

/** Kill switch for summary injection. */
const SUMMARY_ENABLED = process.env.OFFICINA_NO_CLEAR_SUMMARY !== "1";

/** Task file path — updated each turn from the context handler. */
let taskFilePath = ".pi/tasks/default.json";

export default function (pi: ExtensionAPI) {
  const config = clearConfig();
  if (!config.enabled) return; // kill switch — register nothing

  /** toolCallId → target file path (for active-file resolution). */
  const callFiles = new Map<string, string>();

  pi.on("tool_call", async (event) => {
    const e = event as { toolCallId?: string; toolName?: string; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "read" && name !== "read_file" && name !== "grep" && name !== "rg" &&
        name !== "edit" && name !== "write") return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (file && e.toolCallId) callFiles.set(e.toolCallId, file);
  });

  pi.on("tool_result", (event) => {
    const e = event as { toolCallId?: string };
    if (e.toolCallId) callFiles.delete(e.toolCallId);
  });

  pi.on("session_start", (_event, ctx) => {
    const sm = (ctx as { sessionManager?: { getSessionFile?: () => string | null } }).sessionManager;
    const stem = sm?.getSessionFile?.()?.split("/").pop()?.replace(/\.jsonl$/, "") ?? "default";
    taskFilePath = `.pi/tasks/${stem}.json`;
  });

  pi.on("context", async (event, ctx) => {
    tickTurn();
    registerTaskFiles(taskFilePath);

    const resolveFile = (m: unknown): string => {
      if (!isToolResult(m)) return "";
      return callFiles.get((m as ToolResultLike).toolCallId) ?? "";
    };

    const plan = planClear(event.messages, config, isActive, resolveFile);
    if (plan.cleared === 0) return undefined;

    emitHarnessEvent(harnessEvent("lc-clearer", "cleared", { freed_tokens: plan.freedTokens, detail: `${plan.cleared} result(s) stubbed` }));
    ctx.ui.setStatus(
      "tool-clearer",
      `cleared ${plan.cleared} stale tool result(s) (~${plan.freedTokens} tokens)`,
    );

    // Inject summary of cleared results (compaction-resistant knowledge bridge)
    let messages = plan.messages;
    if (SUMMARY_ENABLED && plan.clearedResults.length > 0) {
      const summaries: string[] = [];
      for (const r of plan.clearedResults.slice(0, SUMMARY_MAX)) {
        const s = summarizeResult(r);
        if (s) summaries.push(s);
      }
      if (summaries.length > 0) {
        const summaryTail = {
          role: "custom" as const,
          customType: "lc-clearer-summary",
          content: `\n\n[cleared tool results — key data preserved]\n${summaries.join("\n")}`,
          display: false,
          details: {},
          timestamp: Date.now(),
        };
        messages = [...messages, summaryTail];
      }
    }

    return { messages: messages as never };
  });
}
