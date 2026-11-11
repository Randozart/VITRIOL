// session-ledger — a single, deterministic orientation line kept in the
// conversation. The ledger message is RELOCATED TO THE TAIL on every
// context pass: the previous copy is removed and a fresh one is appended
// after the latest exchange.
//
// Why tail and not in-place upsert (2026-11-11): the engine reuses its KV
// cache only on exact token-prefix continuation — ONE changed token
// truncates reuse from that point. An in-place-updated ledger sits at its
// original near-top injection position and changes every turn, so reuse
// stopped there and the whole conversation tail re-prefilled every turn.
// With the ledger always last, the break point is the ledger itself and
// the shared prefix grows with the conversation; the only extra cost over
// no ledger is the ~10-token ledger block. Same contract as
// _shared/inject.ts (tail-append custom messages; never system-prompt).
//
// Gives a small model a stable "where am I" anchor (message count, ~token
// estimate, files touched) without spending a read on state.
//
// Kill switch: OFFICINA_NO_LEDGER=1 (Rule 15).
//
// Provenance: original work, this repo (First-Party Mandate). Plan:
// .opencode/plans/officina-algorithmic-support-2026-08-31.md (P6);
// tail relocation: .opencode/plans/officina-multisession-hardening-2026-11-11.md (I4).

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

/** True when a message is a ledger custom message. */
export function isLedgerMessage(m: unknown): boolean {
  const x = m as { role?: string; customType?: string };
  return x.role === "custom" && x.customType === LEDGER_CUSTOM_TYPE;
}

/**
 * Relocate the ledger to the tail of a message list: drop every existing
 * ledger message (wherever it sits) and append a fresh one after the
 * latest exchange. Preserves the identity of all non-ledger messages so
 * the engine's prefix cache keeps the conversation up to the ledger
 * position. Pure — never mutates inputs, never accumulates.
 */
export function relocateLedger(
  messages: unknown[],
  block: string,
): unknown[] {
  const kept = messages.filter((m) => !isLedgerMessage(m));
  kept.push({
    role: "custom" as const,
    customType: LEDGER_CUSTOM_TYPE,
    content: "\n\n" + block,
    display: false,
    details: {},
    timestamp: Date.now(),
  });
  return kept;
}
