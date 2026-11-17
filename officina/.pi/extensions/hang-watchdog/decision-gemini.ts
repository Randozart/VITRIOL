// Gemini Decision 1 client for the watchdog lane (2026-11-16).
//
// Why this exists: the free-form judges we tried are either too small to emit
// valid JSON (1B) or verbose and uncalibrated (27B chat). Gemini Decision 1 is a
// *decisions* endpoint: one prefill pass, choice questions, and — the point —
// **calibrated probabilities** per option, with zero output tokens.
//
// Contract: POST https://generativelanguage.googleapis.com/v1beta/decisions
//   auth: header `x-goog-api-key` (the `?key=` form is documented for
//   generateContent, not for this endpoint)
//   body: { model, input, questions: { liveness: { type: "choice", instructions,
//         criteria: { ALIVE, HUNG, UNKNOWN } } } }
//   reply: answers.liveness = { choice, probabilities, confidence } where
//          confidence == p_max, plus usage with zero output tokens.
//
// The mapping below feeds the existing gate in decision.ts, which is the only
// thing allowed to promote a verdict to healthy:
//   answer_confidence = confidence (p_max)
//   action.act_probability = confidence   ← NOT P(HUNG): the gate demands both
//     scores clear the floor, and "act" means "act on this verdict", so using
//     P(HUNG) would reject every ALIVE answer.
//   abstention = "passed" | "abstained"
//
// PROVENANCE: original Officina work (this repo).

import type { DecisionReply } from "./decision.ts";
import { scrubEvidence } from "./decision-redact.ts";

export const DECISIONS_ENDPOINT =
  "https://generativelanguage.googleapis.com/v1beta/decisions";
export const DEFAULT_DECISION_MODEL = "gemini-decision-1";
/** One prefill pass; the watchdog cannot wait long. */
export const DEFAULT_TIMEOUT_MS = 2_500;

export interface DecisionEvidence {
  command: string;
  elapsedS: number;
  stalledS: number;
  outputTail: string;
}

export interface GeminiOptions {
  key: string;
  model?: string;
  endpoint?: string;
  timeoutMs?: number;
  floor: number;
  /** Required p_max − p_second, so a three-way coin toss cannot pass. */
  margin?: number;
  redact?: boolean;
  fetchImpl?: typeof fetch;
}

export type GeminiOutcome =
  | {
      ok: true;
      reply: DecisionReply;
      reason: string;
      probabilities: Record<string, number>;
      /** Round-trip time of the decisions call, for `/decision test` and the ledger. */
      latencyMs?: number;
      status?: number;
    }
  | {
      ok: false;
      reason: string;
      /** HTTP status when the failure came from the service (absent on transport errors). */
      status?: number;
      /** Machine class: auth | model-not-found | invalid-request | rate-limit | safety | service. */
      kind?: DecisionErrorKind;
      latencyMs?: number;
    };

/** Failure classes the operator can act on differently. */
export type DecisionErrorKind =
  | "auth"
  | "model-not-found"
  | "invalid-request"
  | "rate-limit"
  | "safety"
  | "service";

const OPTIONS = {
  ALIVE:
    "clearly progressing: compiling, downloading, iterating, making measurable progress",
  HUNG: "blocked: deadlock, waiting for stdin, infinite repetition, no measurable progress",
  UNKNOWN: "insufficient, ambiguous, stale or contradictory evidence",
} as const;

const SAFETY_CODES = new Set([
  "safety",
  "prohibited_content",
  "blocklist",
  "image_safety",
  "content_blocked",
]);

/** The request body for one liveness decision over shared evidence. */
export function buildDecisionsRequest(
  evidence: DecisionEvidence,
  model = DEFAULT_DECISION_MODEL,
  redact = true,
): Record<string, unknown> {
  const ev = scrubEvidence(evidence.command, evidence.outputTail, redact);
  const input = [
    `Command: ${ev.command}`,
    `Elapsed: ${evidence.elapsedS}s | Stalled: ${evidence.stalledS}s`,
    `Output tail: ${ev.output_tail}`,
  ].join("\n");
  return {
    model,
    input,
    questions: {
      liveness: {
        type: "choice",
        instructions:
          "Judge whether this command is alive, hung, or undecidable from the evidence. Silence alone is not enough.",
        criteria: { ...OPTIONS },
      },
    },
  };
}

/**
 * Reasons a request can never reach the decisions endpoint (measured 2026-11-17):
 * criteria needs 2..255 options, there must be at least one question, and the
 * input text must not be empty. Checked BEFORE the call so an impossible request
 * fails with a name instead of a round-tripped 400.
 */
export function preflightDecision(body: {
  input?: unknown;
  questions?: Record<string, { criteria?: Record<string, unknown> }>;
}): string | null {
  const input = typeof body.input === "string" ? body.input : "";
  if (input.trim().length === 0) return "empty evidence";
  const questions = Object.entries(body.questions ?? {});
  if (questions.length === 0) return "no questions";
  for (const [name, q] of questions) {
    const n = Object.keys(q?.criteria ?? {}).length;
    if (n < 2) return `${name} has ${n} option${n === 1 ? "" : "s"}`;
    if (n > 255) return `${name} has ${n} options (max 255)`;
  }
  return null;
}

interface RawAnswer {
  choice?: unknown;
  confidence?: unknown;
  probabilities?: Record<string, unknown>;
}

function numeric(
  map: Record<string, unknown> | undefined,
): Record<string, number> {
  const out: Record<string, number> = {};
  for (const [k, v] of Object.entries(map ?? {})) {
    if (typeof v === "number" && Number.isFinite(v)) out[k] = v;
  }
  return out;
}

/**
 * Translate a decisions reply into the watchdog contract, applying the floor and
 * the margin. Never promotes a weak answer: abstention is expressed in the
 * reply, and the gate in decision.ts turns it into `unknown`.
 */
export function mapDecisionsReply(
  doc: unknown,
  opts: { floor: number; margin?: number },
): GeminiOutcome {
  const floor = opts.floor;
  const margin = opts.margin ?? 0;
  const answers = (doc as { answers?: Record<string, RawAnswer> } | null)
    ?.answers;
  const answer = answers?.liveness;
  if (!answer) {
    const block = (doc as { promptFeedback?: { blockReason?: unknown } } | null)
      ?.promptFeedback?.blockReason;
    if (typeof block === "string")
      return { ok: false, reason: `safety block (${block})` };
    return { ok: false, reason: "reply has no liveness answer" };
  }
  const probabilities = numeric(answer.probabilities);
  const ranked = Object.entries(probabilities).sort((a, b) => b[1] - a[1]);
  const pmax =
    typeof answer.confidence === "number" && Number.isFinite(answer.confidence)
      ? answer.confidence
      : (ranked[0]?.[1] ?? 0);
  const second = ranked.length > 1 ? ranked[1]![1] : 0;
  // MEASURED (2026-11-16): the endpoint can return choice:"UNKNOWN" while the
  // distribution says ALIVE 0.79 / UNKNOWN 0.16 — i.e. `choice` is not the
  // argmax. The probabilities are the calibrated signal, so the verdict comes
  // from them; the API's own label is kept for the reason string only.
  const apiChoice = typeof answer.choice === "string" ? answer.choice : "";
  const choice = ranked.length > 0 ? ranked[0]![0] : apiChoice;

  const reply = (accepted: boolean, reason: string): GeminiOutcome => ({
    ok: true,
    reason,
    probabilities,
    reply: {
      usage: {
        truncated: false,
        state_tokens_dropped: 0,
        truncated_questions: [],
        options: { liveness: { total: 3, distinct: 3 } },
      },
      answers: {
        liveness: {
          type: "choice",
          choice: accepted ? choice : "UNKNOWN",
          answer_confidence: pmax,
          action: { act_probability: pmax },
          abstention: accepted ? "passed" : "abstained",
          low_confidence: !accepted,
        },
      },
    },
  });

  const label = apiChoice && apiChoice !== choice ? ` (api said ${apiChoice})` : "";
  if (!["ALIVE", "HUNG", "UNKNOWN"].includes(choice)) {
    return reply(false, `unexpected choice ${JSON.stringify(choice)}${label}`);
  }
  if (choice === "UNKNOWN") return reply(false, `argmax is UNKNOWN${label}`);
  if (!(pmax >= floor))
    return reply(false, `below floor (p=${pmax.toFixed(3)})`);
  if (margin > 0 && pmax - second < margin) {
    return reply(
      false,
      `ambiguous (p=${pmax.toFixed(3)} lead=${(pmax - second).toFixed(3)} < ${margin})`,
    );
  }
  return reply(true, `choice ${choice} p=${pmax.toFixed(3)}${label}`);
}

/**
 * Turn a non-2xx decisions response into a reason the operator can act on
 * (2026-11-17). The API names the offending field ("criteria: must have between
 * 2 and 255 options"), and that sentence used to be parsed and thrown away, so
 * every failure read as a bare "HTTP 400" and nothing was recoverable later.
 */
export function classifyDecisionError(
  status: number,
  doc: unknown,
): { kind: DecisionErrorKind; reason: string } {
  const err = (doc as { error?: { code?: unknown; status?: unknown; message?: unknown } } | null)
    ?.error;
  const rawCode = typeof err?.code === "string" ? err.code : String(err?.status ?? "");
  const message =
    typeof err?.message === "string"
      ? err.message.replace(/\s+/g, " ").trim().slice(0, 240)
      : "";
  const detail = message ? `: ${message}` : "";
  if (SAFETY_CODES.has(rawCode)) {
    return { kind: "safety", reason: `safety block (${rawCode})` };
  }
  if (status === 429) return { kind: "rate-limit", reason: "rate limited (429)" };
  if (status === 401 || status === 403) {
    return { kind: "auth", reason: `auth rejected (HTTP ${status})${detail}` };
  }
  if (status === 404 || rawCode === "not_found") {
    return {
      kind: "model-not-found",
      reason: `model not found (HTTP ${status})${detail}`,
    };
  }
  if (status === 400 || rawCode === "invalid_request" || rawCode === "INVALID_ARGUMENT") {
    return {
      kind: "invalid-request",
      reason: `invalid request (HTTP ${status} ${rawCode || "bad_request"})${detail}`,
    };
  }
  return {
    kind: "service",
    reason: `decision service error (HTTP ${status})${detail}`,
  };
}

/** One decisions call. Transport failures are returned, never thrown. */
export async function askGeminiDecision(
  evidence: DecisionEvidence,
  opts: GeminiOptions,
): Promise<GeminiOutcome> {
  const key = String(opts.key ?? "").trim();
  if (!key) return { ok: false, reason: "no gemini key configured" };
  const body = buildDecisionsRequest(
    evidence,
    opts.model ?? DEFAULT_DECISION_MODEL,
    opts.redact ?? true,
  );
  const blocked = preflightDecision(body as Parameters<typeof preflightDecision>[0]);
  if (blocked) {
    return { ok: false, kind: "invalid-request", reason: `not sent: ${blocked}` };
  }
  const startedAt = Date.now();
  const doFetch = opts.fetchImpl ?? fetch;
  const controller = new AbortController();
  const timer = setTimeout(
    () => controller.abort(),
    Math.max(500, opts.timeoutMs ?? DEFAULT_TIMEOUT_MS),
  );
  try {
    const response = await doFetch(opts.endpoint ?? DECISIONS_ENDPOINT, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        // The documented auth form for this endpoint.
        "x-goog-api-key": key,
      },
      body: JSON.stringify(
        buildDecisionsRequest(
          evidence,
          opts.model ?? DEFAULT_DECISION_MODEL,
          opts.redact ?? true,
        ),
      ),
      signal: controller.signal,
    });
    const text = await response.text();
    let doc: unknown = undefined;
    try {
      doc = text ? JSON.parse(text) : undefined;
    } catch {
      return { ok: false, reason: "unusable reply (not JSON)" };
    }
    if (!response.ok) {
      const classified = classifyDecisionError(response.status, doc);
      return {
        ok: false,
        status: response.status,
        kind: classified.kind,
        reason: classified.reason,
        latencyMs: Date.now() - startedAt,
      };
    }
    const mapped = mapDecisionsReply(doc, { floor: opts.floor, margin: opts.margin });
    return {
      ...mapped,
      status: response.status,
      latencyMs: Date.now() - startedAt,
    };
  } catch (error) {
    const name = (error as { name?: string } | undefined)?.name;
    if (name === "AbortError") {
      return { ok: false, reason: "timeout", latencyMs: Date.now() - startedAt };
    }
    return { ok: false, reason: "unreachable", latencyMs: Date.now() - startedAt };
  } finally {
    clearTimeout(timer);
  }
}
