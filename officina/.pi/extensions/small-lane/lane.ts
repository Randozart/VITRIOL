// Small-lane config resolution (pure; testable).
//
// M1 of docs/CRUSH-MINING-PLAN-2026-08-31.md (2026-08-31): Crush's snappiness
// leans on a small local model doing titles/summaries/compaction while the big
// model only handles agent turns. This ext routes pi's compaction summarization
// to the VITRIOL small lane (mellum2 on :8287) instead of the 27B master.
//
// Tuning / kill switches:
//   LITTLE_CODER_NO_SMALL_LANE=1        hard off (default compaction on the main model)
//   LITTLE_CODER_SMALL_LANE_URL         base URL (default http://127.0.0.1:8287/v1)
//   LITTLE_CODER_SMALL_LANE_MODEL       model id (default mellum2-instruct)
//   LITTLE_CODER_SMALL_LANE_CTX         context window registered (default 131072)

export interface LaneConfig {
  enabled: boolean;
  baseUrl: string;
  modelId: string;
  contextWindow: number;
}

const DEFAULT_URL = "http://127.0.0.1:8287/v1";
const DEFAULT_MODEL = "mellum2-instruct";
const DEFAULT_CTX = 131072;

export function resolveLaneConfig(env: NodeJS.ProcessEnv = process.env): LaneConfig {
  if (env.LITTLE_CODER_NO_SMALL_LANE === "1") {
    return { enabled: false, baseUrl: DEFAULT_URL, modelId: DEFAULT_MODEL, contextWindow: DEFAULT_CTX };
  }
  const rawUrl = env.LITTLE_CODER_SMALL_LANE_URL?.trim();
  const rawModel = env.LITTLE_CODER_SMALL_LANE_MODEL?.trim();
  const rawCtx = Number(env.LITTLE_CODER_SMALL_LANE_CTX);
  return {
    enabled: true,
    baseUrl: rawUrl || DEFAULT_URL,
    modelId: rawModel || DEFAULT_MODEL,
    contextWindow: Number.isFinite(rawCtx) && rawCtx > 0 ? rawCtx : DEFAULT_CTX,
  };
}

// The summarizer prompt (compaction is the one job the small model MUST do
// well — it replaces conversation history, so the prompt is strict).
export const SUMMARY_PROMPT = `You are a conversation summarizer. Create a comprehensive summary of this conversation that captures:

1. The main goals and objectives discussed
2. Key decisions made and their rationale
3. Important code changes, file modifications, or technical details
4. Current state of any ongoing work
5. Any blockers, issues, or open questions
6. Next steps that were planned or suggested

Be thorough but concise. The summary will replace the conversation history, so include all information needed to continue the work effectively.

Format the summary as structured markdown with clear sections.

<conversation>
{{CONVERSATION}}
</conversation>`;

export function buildSummaryText(conversationText: string, previousSummary?: string): string {
  const prev = previousSummary ? `\n\nPrevious session summary for context:\n${previousSummary}` : "";
  return SUMMARY_PROMPT.replace("captures:", `captures:${prev}`).replace("{{CONVERSATION}}", conversationText);
}

// Decision after the lane call returns: use the summary, or fall back to pi's
// default compaction on the main model? Empty/whitespace output falls back —
// a blank summary silently destroying history would be worse than no lane.
export function laneCompactionOutcome(summary: string | null | undefined): "use" | "fallback" {
  return summary && summary.trim() ? "use" : "fallback";
}

/** chars/4 estimate over the messages a compaction must summarize. */
export function estimateMessagesTokens(messages: ReadonlyArray<unknown>): number {
  let chars = 0;
  for (const raw of messages) {
    const m = raw as { content?: unknown };
    const content = m?.content;
    if (typeof content === "string") {
      chars += content.length;
    } else if (Array.isArray(content)) {
      for (const part of content) {
        const p = part as { text?: unknown; thinking?: unknown; input?: unknown };
        if (typeof p?.text === "string") chars += p.text.length;
        else if (typeof p?.thinking === "string") chars += p.thinking.length;
        else if (p?.input !== undefined) chars += JSON.stringify(p.input).length;
      }
    }
  }
  return Math.ceil(chars / 4);
}

export interface SummarizerCandidate {
  provider: string;
  id: string;
  contextWindow: number;
}

/**
 * Fit-aware summarizer choice (2026-11-14): compaction requests carry the
 * WHOLE transcript, so the summarizer must have a window >= estimate +
 * reserve. Picks the largest window (cloud first for speed; local ties as
 * the safe fallback). Pure.
 */
export function orderSummarizers(
  candidates: ReadonlyArray<SummarizerCandidate>,
  estimateTokens: number,
  reserveTokens = 4096,
): SummarizerCandidate[] {
  const need = estimateTokens + reserveTokens;
  const fit = candidates.filter((c) => Number(c.contextWindow) >= need);
  fit.sort((a, b) => {
    const byWindow = Number(b.contextWindow) - Number(a.contextWindow);
    if (byWindow !== 0) return byWindow;
    const localA = a.provider === "llamacpp" ? 1 : 0;
    const localB = b.provider === "llamacpp" ? 1 : 0;
    if (localA !== localB) return localA - localB; // cloud first on ties
    return a.id.localeCompare(b.id);
  });
  return fit;
}

export function chooseSummarizer(
  candidates: ReadonlyArray<SummarizerCandidate>,
  estimateTokens: number,
  reserveTokens = 4096,
): SummarizerCandidate | undefined {
  return orderSummarizers(candidates, estimateTokens, reserveTokens)[0];
}
