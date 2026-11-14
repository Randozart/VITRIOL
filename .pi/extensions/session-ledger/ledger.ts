// Session orientation statistics are recorded as durable append-only snapshots.
// Boundary updates retain every earlier snapshot for exact prefix reuse;
// statistics exclude earlier ledgers so the ledger does not count itself.
// Kill switch: OFFICINA_NO_LEDGER=1.

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
  /** Number of non-system, non-ledger messages at the snapshot boundary. */
  messages: number;
  /** Rough text token estimate at that boundary, excluding system/ledger (chars / 4). */
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
