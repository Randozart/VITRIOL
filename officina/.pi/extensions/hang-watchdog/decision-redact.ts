// Redaction for outbound decision evidence (2026-11-16).
//
// The watchdog's evidence is a command line plus an output tail. Those routinely
// contain secrets: env dumps, bearer tokens, connection strings, keys pasted
// into a chat, URLs with credentials. When the decision lane is a remote model
// (Gemini Decision 1), that text leaves the box — so it is scrubbed first and
// the unredacted payload is never logged.
//
// PROVENANCE: original Officina work (this repo).
//
// Invariants:
//   R1 — a known key shape never survives (Google/HF/GitHub/OpenAI/Slack/JWT).
//   R2 — assignments (*_KEY=, *_TOKEN=, PASSWORD=, AUTH=) lose their value.
//   R3 — URL credentials and secret query params are replaced, not truncated.
//   R4 — output is bounded, so a huge tail cannot blow the token budget.
//   R5 — redaction is idempotent: redacting twice yields the same string.

/** The single marker used for anything scrubbed. */
export const REDACTED = "«redacted»";

/** Cap on the command text sent to a judge. */
export const MAX_COMMAND = 200;
/** Cap on the output tail sent to a judge. */
export const MAX_TAIL = 200;
/** Cap on the whole `input` string sent to a judge. */
export const MAX_TOTAL = 1200;

interface Rule {
  pattern: RegExp;
  replace: string | ((match: string, ...rest: string[]) => string);
}

/**
 * Ordered rules. Specific token shapes first, then assignments, then generic
 * blobs — order matters, because the generic rule would otherwise eat the
 * `KEY=` prefix that the assignment rule needs to recognise.
 */
const RULES: Rule[] = [
  // Google API keys (AIza + 35 chars).
  { pattern: /\bAIza[0-9A-Za-z_-]{20,}\b/g, replace: REDACTED },
  // OpenAI-style and Anthropic-style keys.
  { pattern: /\bsk-[A-Za-z0-9_-]{16,}\b/g, replace: REDACTED },
  { pattern: /\bsk-ant-[A-Za-z0-9_-]{16,}\b/g, replace: REDACTED },
  // GitHub tokens.
  {
    pattern: /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{16,}\b/g,
    replace: REDACTED,
  },
  // Hugging Face.
  { pattern: /\bhf_[A-Za-z0-9]{16,}\b/g, replace: REDACTED },
  // Slack.
  { pattern: /\bxox[baprs]-[A-Za-z0-9-]{10,}\b/g, replace: REDACTED },
  // AWS access key ids.
  { pattern: /\bAKIA[0-9A-Z]{16}\b/g, replace: REDACTED },
  // Email addresses: never needed to judge liveness.
  {
    pattern: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g,
    replace: REDACTED,
  },
  // URL credentials: scheme://user:pass@host
  {
    pattern: /(\b[a-z][a-z0-9+.-]*:\/\/)[^\s/:@]+:[^\s/@]+@/gi,
    replace: (_m, scheme: string) => `${scheme}${REDACTED}@`,
  },
  // Secret query parameters.
  {
    pattern:
      /([?&](?:key|api[_-]?key|token|access[_-]?token|auth|signature|sig)=)[^&\s"']+/gi,
    replace: (_m, prefix: string) => `${prefix}${REDACTED}`,
  },
  // Assignments: FOO_API_KEY=..., DB_PASSWORD=..., x-auth-token: ...
  {
    pattern:
      /(\b[A-Za-z0-9_.-]*(?:KEY|TOKEN|SECRET|PASSWORD|PASSWD|PWD|AUTH|CREDENTIAL)\b["']?\s*[:=]\s*)("[^"]*"|'[^']*'|\S+)/gi,
    replace: (_m, prefix: string) => `${prefix}${REDACTED}`,
  },
  // Bearer/basic credentials in prose or headers.
  {
    pattern: /\b(Bearer|Basic|Token)\s+[A-Za-z0-9._~+/=-]{12,}/gi,
    replace: (_m, scheme: string) => `${scheme} ${REDACTED}`,
  },
  // JWTs (three dot-separated base64url segments).
  {
    pattern: /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/g,
    replace: REDACTED,
  },
  // Raw hex dumps (md5/sha fragments and longer).
  { pattern: /\b[0-9a-fA-F]{32,}\b/g, replace: REDACTED },
  // Long base64-ish blobs (mixed case + digits, long enough to be a key).
  {
    pattern:
      /\b(?=[A-Za-z0-9+/=_-]{40,}\b)(?=[^\s]*[A-Z])(?=[^\s]*[a-z])(?=[^\s]*[0-9])[A-Za-z0-9+/=_-]+\b/g,
    replace: REDACTED,
  },
];

/** Scrub one string. Pure; idempotent (R5). */
export function redactText(text: string): string {
  if (!text) return "";
  let out = text;
  for (const rule of RULES) {
    out = out.replace(rule.pattern, rule.replace as never);
  }
  return out;
}

/** Scrub, then cap. The order matters: scrub first so a key split by the cap
 *  cannot survive as a recognisable fragment. */
export function redactCapped(text: string, max: number): string {
  const clean = redactText(text);
  if (clean.length <= max) return clean;
  return `${clean.slice(0, Math.max(0, max - 1))}…`;
}

/** Evidence as it will be sent: redacted and bounded (R4). */
export interface OutboundEvidence {
  command: string;
  output_tail: string;
  redacted: boolean;
}

/**
 * Prepare evidence for a remote judge.
 * `enabled === false` returns the text untouched (local-only mode) but still
 * capped, so the caller cannot accidentally send an unbounded tail.
 */
export function scrubEvidence(
  command: string,
  tail: string,
  enabled = true,
): OutboundEvidence {
  if (!enabled) {
    return {
      command: capOnly(command, MAX_COMMAND),
      output_tail: capOnly(tail, MAX_TAIL),
      redacted: false,
    };
  }
  const cmd = redactCapped(command, MAX_COMMAND);
  const rest = Math.max(0, MAX_TOTAL - cmd.length);
  return {
    command: cmd,
    output_tail: redactCapped(tail, Math.min(MAX_TAIL, rest)),
    redacted: true,
  };
}

function capOnly(text: string, max: number): string {
  if (text.length <= max) return text;
  return `${text.slice(0, Math.max(0, max - 1))}…`;
}
