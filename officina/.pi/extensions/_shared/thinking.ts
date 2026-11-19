// Per-lane thinking, always with a budget (2026-11-19).
//
// The engine turned out to have two separate switches and only one of them was
// ever set:
//
//   enable_thinking      -- whether the model thinks at all (works per request)
//   reasoning_budget     -- how many tokens it may spend (NEVER set by anyone)
//
// With the budget unset the engine does not even construct its budget sampler
// (`common/sampling.cpp:311`), so nothing can force `</think>` and the thinker
// spends the answer's `n_predict` whole. Measured, one request each:
//
//   enable_thinking=true, no budget  -> finish_reason=length, content "" (64/64 tok)
//   enable_thinking=true, budget=24  -> finish_reason=stop,  content "Red, blue, and yellow."
//
// So this module never turns thinking on without also setting a budget. They are
// one decision, not two: a marker that enables thinking alone is the same bug with
// better ergonomics.
//
// The prompt asks for it:
//
//   system: "Judge these findings carefully. [[officina-think]]"
//   system: "Deep recall over the transcript. [[officina-think 2048]]"

/** `[[officina-think]]` or `[[officina-think N]]`. */
const MARKER = /\[\[officina-think(?:\s+(\d+))?\]\]/gi;

/** Thinking budget used when the marker names none. */
export const DEFAULT_THINKING_BUDGET = 1024;

/** Answer room reserved even when the caller's max_tokens is tiny. */
export const MIN_ANSWER_ROOM = 256;

/** Floor for a budget. A cap of 0 would be "never think", which the marker contradicts. */
export const MIN_THINKING_BUDGET = 32;

/** Opt-in switch, default off (same policy as the other marker features). */
export function thinkingEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.OFFICINA_THINKING === "1";
}

/** All message text in a chat payload, for marker detection. */
function payloadText(payload: Record<string, unknown>): string {
  const messages = payload.messages;
  if (!Array.isArray(messages)) return "";
  return messages
    .map((m) => {
      const content = (m as { content?: unknown })?.content;
      return typeof content === "string" ? content : "";
    })
    .join("\n");
}

/** Budget requested by a marker, or null when the prompt carries none. */
export function requestedBudget(
  payload: Record<string, unknown>,
): number | null {
  const text = payloadText(payload);
  MARKER.lastIndex = 0;
  const match = MARKER.exec(text);
  if (!match) return null;
  const explicit = match[1]
    ? Number.parseInt(match[1], 10)
    : DEFAULT_THINKING_BUDGET;
  return Number.isFinite(explicit) ? explicit : DEFAULT_THINKING_BUDGET;
}

/** Answer budget the request may spend after thinking, from max_tokens. */
export function answerRoom(payload: Record<string, unknown>): number {
  const raw = payload.max_tokens ?? payload.max_completion_tokens;
  const max = typeof raw === "number" && Number.isFinite(raw) ? raw : 0;
  return Math.max(0, max - MIN_ANSWER_ROOM);
}

/**
 * Clamp a requested thinking budget so the answer still has room.
 *
 * This is the whole point: the cap is derived from the SAME number the answer
 * draws from. It is never -1 (unrestricted) and never larger than what is left.
 */
export function clampBudget(requested: number, room: number): number {
  const upper = Math.max(MIN_THINKING_BUDGET, room);
  return Math.min(Math.max(requested, MIN_THINKING_BUDGET), upper);
}

/** Strip the marker from every message. */
function stripMarkers(
  payload: Record<string, unknown>,
): Record<string, unknown> {
  const messages = payload.messages;
  if (!Array.isArray(messages)) return payload;
  return {
    ...payload,
    messages: messages.map((m) => {
      const content = (m as { content?: unknown })?.content;
      if (typeof content !== "string") return m;
      return {
        ...(m as object),
        content: content.replace(MARKER, "").replace(/\s+$/, ""),
      };
    }),
  };
}

/**
 * Apply the thinking request when the prompt asked for it.
 *
 * Returns the payload unchanged when: the feature is off, no marker is present,
 * or the caller already chose its own `enable_thinking` (an explicit caller choice
 * always wins — including "off", which must stay off).
 */
export function applyThinkingInstruction(
  payload: Record<string, unknown>,
  env: NodeJS.ProcessEnv = process.env,
): Record<string, unknown> {
  if (!thinkingEnabled(env)) return payload;
  const requested = requestedBudget(payload);
  if (requested === null) return payload;

  const existing = payload.chat_template_kwargs;
  if (
    existing !== undefined &&
    typeof existing === "object" &&
    existing !== null
  ) {
    if ("enable_thinking" in (existing as Record<string, unknown>))
      return payload;
  }

  const budget = clampBudget(requested, answerRoom(payload));
  return {
    ...stripMarkers(payload),
    chat_template_kwargs: {
      ...((existing as Record<string, unknown> | undefined) ?? {}),
      enable_thinking: true,
    },
    // Paired with the flag above in the same object: they must never diverge.
    reasoning_budget_tokens: budget,
  };
}
