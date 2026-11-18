// Structured output for the lanes we consume as data (2026-11-18).
//
// Measured (density-bench, 2026-11-18): a nested schema is where an unconstrained
// model fails and wastes tokens — baseline produced pretty-printed JSON with
// `rule_id` instead of `rule` (INVALID, 533 chars); with `response_format`
// json_schema it produced 310 chars, valid, three runs out of three. A *simple*
// flat schema showed the opposite of a win (+13 % tokens, both valid), because
// the model already complies there.
//
// So: constrain only lanes whose payload is machine-consumed AND whose schema is
// non-trivial, and let the prompt say which schema it wants. The marker keeps the
// decision with the code that writes the prompt:
//
//   system: "… Answer ONLY with JSON. [[officina-json findings]]"
//
// The provider attaches the schema; nothing else in the request changes.

/** Marker a prompt uses to request a constrained reply. */
const MARKER = /\[\[officina-json\s+([a-z0-9_-]+)\]\]/i;

/** Schemas the harness is allowed to constrain, by marker name. */
export const SCHEMAS: Record<string, unknown> = {
  /** Verdict lanes: judge, decision lane, route verdict. */
  decision: {
    type: "object",
    properties: {
      verdict: { type: "string" },
      lane: { type: "string" },
      confidence: { type: "number" },
      why: { type: "string" },
    },
    required: ["verdict", "lane", "confidence", "why"],
    additionalProperties: false,
  },
  /** Findings lanes: review, verify, audit (nested — where compliance breaks). */
  findings: {
    type: "object",
    properties: {
      findings: {
        type: "array",
        items: {
          type: "object",
          properties: {
            rule: { type: "string" },
            severity: { type: "string", enum: ["low", "medium", "high"] },
            detail: { type: "string" },
          },
          required: ["rule", "severity", "detail"],
          additionalProperties: false,
        },
      },
      summary: { type: "string" },
    },
    required: ["findings", "summary"],
    additionalProperties: false,
  },
  /** Subagent reports: a fixed shape so the parent can parse them. */
  report: {
    type: "object",
    properties: {
      summary: { type: "string" },
      files: { type: "array", items: { type: "string" } },
      done: { type: "boolean" },
    },
    required: ["summary", "files", "done"],
    additionalProperties: false,
  },
};

/** Kill switch / opt-in. Default OFF until a second measurement confirms E1. */
export function structuredOutputEnabled(
  env: NodeJS.ProcessEnv = process.env,
): boolean {
  return env.OFFICINA_STRUCTURED_OUTPUT === "1";
}

/** Schema name requested by this prompt text, if any. */
export function requestedSchema(text: string): string | null {
  const match = MARKER.exec(text);
  if (!match) return null;
  const name = match[1].toLowerCase();
  return name in SCHEMAS ? name : null;
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

/**
 * Attach `response_format` when the prompt asked for it.
 *
 * Returns the payload unchanged when: the feature is off, no marker is present,
 * the marker names an unknown schema, or the payload already carries a
 * `response_format` (an explicit caller choice always wins).
 */
export function attachStructuredOutput(
  payload: Record<string, unknown>,
  env: NodeJS.ProcessEnv = process.env,
): Record<string, unknown> {
  if (!structuredOutputEnabled(env)) return payload;
  if (payload.response_format !== undefined) return payload;
  const name = requestedSchema(payloadText(payload));
  if (!name) return payload;
  return {
    ...payload,
    response_format: {
      type: "json_schema",
      json_schema: { name, strict: true, schema: SCHEMAS[name] },
    },
  };
}

/** The marker to put in a system prompt that wants a constrained reply. */
export function marker(name: keyof typeof SCHEMAS): string {
  return `[[officina-json ${name}]]`;
}
