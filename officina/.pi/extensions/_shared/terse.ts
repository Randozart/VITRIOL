// Terse output for internal lanes (2026-11-19).
//
// Measured (density-bench, live engine `spec=mtp,ngram-simple:2`, 3 repeats, recall
// guard 3/3 on every attempted arm):
//
//   internal-plan   828 tokens baseline -> 256 (+1 instruction)   = -69.1 %
//   deep-recall     baseline            -> -47.9 %
//
// The same experiment taught a codebook (`⇒ because, ¬ not …`) and it lost: 517
// tokens, i.e. -37.6 %, because the legend costs prompt tokens and does not make
// the model compress harder. So the shipped thing is the cheap half — one
// sentence, no notation, nothing to decode.
//
// The prompt decides, as with `[[officina-json]]`:
//
//   system: "You are planning an engine change. [[officina-terse]]"
//
// A caller that already wrote its own brevity instruction keeps it: the marker is
// removed, and the instruction is only appended when the system message does not
// already ask for terseness.

/** Marker a prompt uses to ask for terse output. */
const MARKER = /\[\[officina-terse\]\]/gi;

/** The instruction measured in E2, verbatim — changing it invalidates the number. */
export const TERSE_INSTRUCTION =
  "Answer as tersely as possible: no articles, no filler, one clause per line.";

/** Opt-in switch, default off (same policy as structured output). */
export function terseEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.OFFICINA_TERSE === "1";
}

/** True when the text already asks for brevity in its own words. */
export function alreadyTerse(text: string): boolean {
  return /\b(terse|tersely|concise|concisely|briefly|no filler)\b/i.test(text);
}

/** Does any message carry the marker? */
export function hasTerseMarker(payload: Record<string, unknown>): boolean {
  const messages = payload.messages;
  if (!Array.isArray(messages)) return false;
  return messages.some((m) => {
    const content = (m as { content?: unknown })?.content;
    return (
      typeof content === "string" && /\[\[officina-terse\]\]/i.test(content)
    );
  });
}

/**
 * Apply the terse instruction when the prompt asked for it.
 *
 * Returns the payload unchanged when: the feature is off, no marker is present, or
 * the system message already asks for brevity (the caller's wording wins, and a
 * second near-identical sentence would just cost tokens).
 */
export function applyTerseInstruction(
  payload: Record<string, unknown>,
  env: NodeJS.ProcessEnv = process.env,
): Record<string, unknown> {
  if (!terseEnabled(env)) return payload;
  if (!hasTerseMarker(payload)) return payload;
  const messages = payload.messages as Array<{
    role?: string;
    content?: unknown;
  }>;
  const systemIndex = messages.findIndex(
    (m) => m?.role === "system" && typeof m.content === "string",
  );
  // Test the caller's wording with the marker removed: the marker itself contains
  // the word "terse", which would otherwise make every marked prompt look as if the
  // caller had already asked for brevity (found by terse.test.ts).
  const alreadyAsked =
    systemIndex >= 0 &&
    alreadyTerse(String(messages[systemIndex].content).replace(MARKER, ""));

  const stripped = messages.map((m) =>
    typeof m?.content === "string"
      ? { ...m, content: m.content.replace(MARKER, "").replace(/\s+$/, "") }
      : m,
  );

  if (alreadyAsked) return { ...payload, messages: stripped };

  if (systemIndex < 0) {
    // No system message: add one rather than mutating a user turn (user text is
    // the operator's, and rewriting it would leak into transcripts).
    return {
      ...payload,
      messages: [{ role: "system", content: TERSE_INSTRUCTION }, ...stripped],
    };
  }
  const withInstruction = stripped.map((m, i) =>
    i === systemIndex
      ? { ...m, content: `${String(m.content).trim()} ${TERSE_INSTRUCTION}` }
      : m,
  );
  return { ...payload, messages: withInstruction };
}
