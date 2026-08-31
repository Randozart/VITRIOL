import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";

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
// Tuning / opt-out:
//   LITTLE_CODER_NO_CLEAR_TOOL_RESULTS=1   hard off (kill switch, Rule 15)
//   LITTLE_CODER_CLEAR_KEEP=<n>            tool results kept verbatim (default 4)
//   LITTLE_CODER_CLEAR_EXCLUDE=a,b         extra tool names never cleared
//                                          (merged onto DEFAULT_EXCLUDE)

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
  const keep = keepRaw !== undefined && keepRaw.trim() !== "" ? Number(keepRaw) : 4;
  const extra = (env.LITTLE_CODER_CLEAR_EXCLUDE ?? "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter((s) => s.length > 0);
  return {
    enabled: env.LITTLE_CODER_NO_CLEAR_TOOL_RESULTS !== "1",
    keep: Number.isFinite(keep) && keep >= 1 ? Math.floor(keep) : 4,
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

/** The result of planning a clear pass over a message list. */
export interface ClearPlan {
  /** Messages to send to the model (a new array when anything changed). */
  messages: unknown[];
  /** Number of tool results replaced with stubs. */
  cleared: number;
  /** Estimated tokens freed this pass. */
  freedTokens: number;
}

/**
 * Plan which tool results to clear, given the config. Pure — unit-testable.
 *
 * Keeps the last `keep` tool results verbatim (the model still needs recent
 * results to reason about), replaces everything older with a stub, and never
 * touches non-tool messages or the message order. Never clears error results
 * (errors are load-bearing for the loop-breaker) — they stay verbatim.
 */
export function planClear(
  messages: unknown[],
  config: ClearConfig,
): ClearPlan {
  if (!config.enabled) return { messages, cleared: 0, freedTokens: 0 };

  // Collect the tool-result indices in order; keep the last `keep` of them.
  // Excluded tools (plan/todo/state) are never candidates — not cleared and
  // not counted toward the keep window, so protection is absolute.
  const resultIndices: number[] = [];
  for (let i = 0; i < messages.length; i++) {
    if (isToolResult(messages[i]) && !isExcluded(messages[i], config)) resultIndices.push(i);
  }
  const clearThreshold = resultIndices.length - config.keep;
  if (clearThreshold <= 0) return { messages, cleared: 0, freedTokens: 0 };

  const out = messages.slice();
  let cleared = 0;
  let freedTokens = 0;
  for (let k = 0; k < clearThreshold; k++) {
    const idx = resultIndices[k];
    const original = out[idx] as ToolResultLike;
    if (original.isError) continue; // errors are never cleared
    const tokens = estimateTokens(original.content ?? []);
    out[idx] = {
      ...original,
      content: [{ type: "text", text: stubFor(original, tokens) }],
    };
    cleared += 1;
    freedTokens += tokens;
  }
  return { messages: out, cleared, freedTokens };
}

export default function (pi: ExtensionAPI) {
  const config = clearConfig();
  if (!config.enabled) return; // kill switch — register nothing

  pi.on("context", async (event, ctx) => {
    const plan = planClear(event.messages, config);
    if (plan.cleared === 0) return undefined;
    emitHarnessEvent(harnessEvent("lc-clearer", "cleared", { freed_tokens: plan.freedTokens, detail: `${plan.cleared} result(s) stubbed` }));
    ctx.ui.setStatus(
      "tool-clearer",
      `cleared ${plan.cleared} stale tool result(s) (~${plan.freedTokens} tokens)`,
    );
    return { messages: plan.messages as never };
  });
}