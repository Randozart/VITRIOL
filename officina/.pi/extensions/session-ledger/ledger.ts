// session-ledger — a single, deterministic orientation line kept in the
// conversation. Unlike the other injectors it does NOT append: the ledger
// message is REPLACED in place on every context pass, so it stays one line
// forever and never accumulates.
//
// Gives a small model a stable "where am I" anchor (message count, ~token
// estimate, files touched) without spending a read on state.
//
// Provenance: original work, this repo (First-Party Mandate). Plan:
// .opencode/plans/officina-algorithmic-support-2026-08-31.md (P6).

export const LEDGER_CUSTOM_TYPE = "lc-ledger";

export interface LedgerConfig {
  enabled: boolean;
  /** Show at most this many recently touched files. */
  maxFiles: number;
}

export function ledgerConfig(env: NodeJS.ProcessEnv = process.env): LedgerConfig {
  return {
    enabled: env.OFFICINA_NO_LEDGER !== "1",
    maxFiles: 4,
  };
}

export interface LedgerStats {
  /** Number of messages in the outgoing context. */
  messages: number;
  /** Rough token estimate over the whole outgoing context (chars / 4). */
  approxTokens: number;
  /** Recently touched files, most recent last. */
  files: string[];
  /** Completed edit/write operations this session. */
  edits: number;
}

/**
 * Render the one-line ledger. Pure.
 */
export function renderLedger(stats: LedgerStats, maxFiles: number): string {
  const files = stats.files.slice(-maxFiles);
  const filePart = files.length > 0 ? ` files=[${files.join(", ")}]` : "";
  return (
    `[ledger: msgs=${stats.messages} ctx≈${fmtK(stats.approxTokens)} tok ` +
    `edits=${stats.edits}${filePart}]`
  );
}

function fmtK(n: number): string {
  return n >= 10_000 ? `${Math.round(n / 1000)}k` : String(n);
}

/**
 * Total character payload of a message list (text content only). Pure.
 */
export function contextChars(messages: Array<{ content?: unknown }>): number {
  let chars = 0;
  for (const m of messages) {
    const c = m.content;
    if (typeof c === "string") {
      chars += c.length;
    } else if (Array.isArray(c)) {
      for (const b of c) {
        if (b && typeof b === "object" && (b as { type?: string }).type === "text") {
          chars += String((b as { text?: string }).text ?? "").length;
        }
      }
    }
  }
  return chars;
}

/**
 * Upsert the ledger message into a message list: replaces the content of
 * the existing ledger message in place (new array + new message object),
 * or appends one when absent. Pure — never mutates inputs.
 */
export function upsertLedger(
  messages: unknown[],
  block: string,
): unknown[] {
  const out = [...messages];
  for (let i = out.length - 1; i >= 0; i--) {
    const m = out[i] as { role?: string; customType?: string };
    if (m.role === "custom" && m.customType === LEDGER_CUSTOM_TYPE) {
      out[i] = { ...(m as object), content: "\n\n" + block, timestamp: Date.now() };
      return out;
    }
  }
  out.push({
    role: "custom" as const,
    customType: LEDGER_CUSTOM_TYPE,
    content: "\n\n" + block,
    display: false,
    details: {},
    timestamp: Date.now(),
  });
  return out;
}
