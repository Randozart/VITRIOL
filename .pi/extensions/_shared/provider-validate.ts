// Provider response shape validation (2026-11-14) — PURE.
//
// "Guaranteeing Pi doesn't have bugs" starts with refusing to trust provider
// payloads: a malformed completion must become a typed violation, not a crash
// or a silent mis-parse. These validators are intentionally conservative —
// they accept unknown extra fields and only reject structural defects.

export interface Violation {
  code: string;
  detail: string;
}

export interface Validation {
  ok: boolean;
  violations: Violation[];
}

const FINISH_REASONS = new Set([
  "stop",
  "length",
  "tool_calls",
  "content_filter",
  "function_call",
  "end_turn",
  "tool_use",
  "max_tokens",
]);

function num(value: unknown): boolean {
  return typeof value === "number" && Number.isFinite(value);
}

/** OpenAI-compatible non-streaming chat completion. */
export function validateChatCompletion(body: unknown): Validation {
  const violations: Violation[] = [];
  if (!body || typeof body !== "object") {
    return { ok: false, violations: [{ code: "not-object", detail: "body is not an object" }] };
  }
  const doc = body as Record<string, unknown>;
  const choices = doc.choices;
  if (!Array.isArray(choices) || choices.length === 0) {
    violations.push({ code: "choices", detail: "choices missing or empty" });
  } else {
    for (const [index, raw] of choices.entries()) {
      const choice = raw as Record<string, unknown>;
      if (!choice || typeof choice !== "object") {
        violations.push({ code: "choice-shape", detail: `choice ${index} is not an object` });
        continue;
      }
      const message = choice.message as Record<string, unknown> | undefined;
      if (!message || typeof message !== "object" || typeof message.role !== "string") {
        violations.push({ code: "message", detail: `choice ${index} has no message.role` });
      }
      const finish = choice.finish_reason;
      if (finish !== null && finish !== undefined && !FINISH_REASONS.has(String(finish))) {
        violations.push({
          code: "finish-reason",
          detail: `choice ${index} finish_reason '${String(finish)}' unknown`,
        });
      }
    }
  }
  const usage = doc.usage;
  if (usage !== undefined && usage !== null) {
    const u = usage as Record<string, unknown>;
    if (typeof u !== "object") {
      violations.push({ code: "usage-shape", detail: "usage is not an object" });
    } else {
      for (const field of ["prompt_tokens", "completion_tokens", "total_tokens"]) {
        if (u[field] !== undefined && !num(u[field])) {
          violations.push({ code: "usage-number", detail: `usage.${field} is not a number` });
        }
      }
    }
  }
  return { ok: violations.length === 0, violations };
}

/** OpenAI-compatible streaming chunk (choices[].delta). */
export function validateStreamChunk(body: unknown): Validation {
  const violations: Violation[] = [];
  if (!body || typeof body !== "object") {
    return { ok: false, violations: [{ code: "not-object", detail: "chunk is not an object" }] };
  }
  const doc = body as Record<string, unknown>;
  const choices = doc.choices;
  if (!Array.isArray(choices)) {
    violations.push({ code: "choices", detail: "chunk choices is not an array" });
  } else {
    for (const [index, raw] of choices.entries()) {
      const choice = raw as Record<string, unknown>;
      if (!choice || typeof choice !== "object") {
        violations.push({ code: "choice-shape", detail: `chunk choice ${index} invalid` });
        continue;
      }
      const delta = choice.delta;
      if (delta !== undefined && (typeof delta !== "object" || delta === null)) {
        violations.push({ code: "delta", detail: `chunk choice ${index} delta invalid` });
      }
    }
  }
  return { ok: violations.length === 0, violations };
}

/** True when a payload looks like one of the shapes we validate. */
export function looksLikeCompletion(body: unknown): boolean {
  return !!body && typeof body === "object" && "choices" in (body as Record<string, unknown>);
}

export function renderViolations(violations: readonly Violation[], limit = 3): string {
  return violations
    .slice(0, limit)
    .map((v) => `${v.code}: ${v.detail}`)
    .join("; ");
}
