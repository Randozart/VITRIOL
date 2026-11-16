// PROVENANCE: original Officina adapter for the documented Jev/Laya System One API.
// The configured endpoint is advisory. No actor KV or generated decision text is used.
import type { WatchedCall } from "./judge.ts";
import { resolveDecisionConfig } from "./config.ts";

/** Where a watchdog verdict came from (2026-11-16): the typed decision lane,
 *  the free cloud lane, or unavailable when both could not answer. */
export type DecisionSource = "decision" | "lane" | "unavailable";

export interface DecisionVerdict {
  alive: boolean | null;
  reason: string;
  source: DecisionSource;
}
export const WATCHDOG_SCHEMA = "watchdog-liveness-v1";
export const QUESTIONS = {
  liveness: {
    type: "choice",
    instructions:
      "Judge this command using only supplied task-scoped evidence. Silence alone is insufficient; abstain when uncertain.",
    criteria: {
      ALIVE:
        "Evidence of this command progressing or legitimately performing finite work",
      HUNG: "Evidence of a blocked prompt, deadlock, or non-terminating repetition",
      UNKNOWN: "Insufficient, ambiguous, stale or incomplete evidence",
    },
  },
};
const unknown = (reason: string): DecisionVerdict => ({
  alive: null,
  reason,
  source: "decision",
});

interface DecisionReply {
  usage?: {
    truncated?: unknown;
    truncated_questions?: unknown;
    state_tokens_dropped?: number;
    options?: Record<string, { total?: number; distinct?: number }>;
  };
  answers?: {
    liveness?: {
      type?: string;
      choice?: string;
      action?: { act_probability?: number };
      answer_confidence?: number;
      abstention?: string;
      low_confidence?: boolean;
    };
  };
  abstention?: string;
}

/** Coverage must include the state and all three distinguishable answer options. */
function completeCoverage(usage: DecisionReply["usage"]): boolean {
  if (!usage || usage.truncated !== false || usage.state_tokens_dropped !== 0)
    return false;
  if (
    !Array.isArray(usage.truncated_questions) ||
    usage.truncated_questions.length > 0
  )
    return false;
  const options = usage.options?.liveness;
  return !options || (options.total === 3 && options.distinct === 3);
}

function passesScore(score: unknown, minimum: number): score is number {
  return (
    typeof score === "number" &&
    Number.isFinite(score) &&
    score >= minimum &&
    score <= 1
  );
}

/** Reject abstention, head/coverage loss and unusable scores, never promoting them to health. */
export function decisionVerdict(
  value: unknown,
  minimum: number,
): DecisionVerdict {
  if (!Number.isFinite(minimum) || minimum < 0 || minimum > 1)
    return unknown("invalid decision answer-score floor");
  const doc = value as DecisionReply | null;
  if (!completeCoverage(doc?.usage))
    return unknown("decision input coverage incomplete or unknown");
  const answer = doc?.answers?.liveness;
  const abstained = [doc?.abstention, answer?.abstention].some(
    (state) => state !== undefined && state !== "passed",
  );
  if (abstained || answer?.low_confidence)
    return unknown("decision model abstained");
  if (!passesScore(answer?.action?.act_probability, minimum))
    return unknown("decision action score below configured floor");
  const score = answer?.answer_confidence;
  if (!passesScore(score, minimum))
    return unknown("decision answer score below configured floor");
  if (
    answer?.type !== "choice" ||
    !["ALIVE", "HUNG"].includes(answer.choice ?? "")
  )
    return unknown("decision model returned UNKNOWN or an invalid choice");
  return {
    alive: answer.choice === "ALIVE",
    reason: `decision-model advisory; answer score ${score.toFixed(3)} (domain quality not established)`,
    source: "decision",
  };
}

/** An explicit endpoint replaces the cloud judge; missing/failed endpoints remain UNKNOWN. */
export async function judgeWithDecision(
  call: WatchedCall,
  signal: AbortSignal,
): Promise<DecisionVerdict | undefined> {
  // Env vars win for throwaway runs; otherwise the operator's `/decision`
  // config file (2026-11-16).
  const { endpoint, key, floor } = resolveDecisionConfig();
  if (!endpoint) return undefined;
  if (!Number.isFinite(floor) || floor < 0 || floor > 1)
    return unknown("invalid decision answer-score floor");
  try {
    const url = new URL(endpoint);
    if (!["http:", "https:"].includes(url.protocol))
      return unknown("unsupported decision endpoint protocol");
    const headers: Record<string, string> = {
      "content-type": "application/json",
    };
    if (key) headers.authorization = `Bearer ${key}`;
    const now = Date.now();
    const state = {
      schema: WATCHDOG_SCHEMA,
      command: call.command.slice(0, 300),
      elapsed_s: Math.floor((now - call.startedAt) / 1000),
      stalled_s: Math.floor((now - (call.stalledSince ?? now)) / 1000),
      output_tail: call.tail ?? "",
      evidence_revision: call.generation ?? 0,
    };
    const response = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify({ state, questions: QUESTIONS }),
      signal,
    });
    if (!response.ok)
      return unknown(`decision service unavailable (HTTP ${response.status})`);
    return decisionVerdict(await response.json(), floor);
  } catch {
    return unknown("decision service unreachable or invalid response");
  }
}
