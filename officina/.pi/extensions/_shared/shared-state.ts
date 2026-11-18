// Shared state under ~/.vitriol/officina — classified, on purpose (2026-11-18).
//
// Several Officina sessions run at once in one project, each a separate `pi`
// process. Every path below is therefore a potential cross-session channel. The
// rule this module enforces: a path is either SESSION-KEYED (safe by
// construction), LAST-WRITER-WINS (accepted, because the value is a cache or a
// single operator setting) or APPEND-ONLY (safe because readers tolerate
// duplicates). Anything unclassified fails `shared-state.test.ts`, so new shared
// state becomes a conscious decision instead of an accident.
//
// The test discovers names by scanning the sources for the literal
// `".vitriol", "officina", "<name>"` pattern — the shape every writer uses — and
// diffs the result against this table.

export type StateKind =
  /** The filename carries the session/pid (shards, per-session stamps). */
  | "session-keyed"
  /** One file, one value; concurrent writers clobber. Fine for caches/config. */
  | "last-writer-wins"
  /** Readers tolerate duplicates and re-read the tail. */
  | "append-only"
  /** Operator-authored config; the last editor is the intent. */
  | "user-config";

export interface SharedStateEntry {
  kind: StateKind;
  note: string;
}

export const SHARED_STATE: Record<string, SharedStateEntry> = {
  // ── session-keyed by construction ───────────────────────────────────────
  "routing-registry.d": {
    kind: "session-keyed",
    note: "one shard per TUI pid; readers glob-merge and TTL-prune",
  },
  "compact-on-finish": {
    kind: "session-keyed",
    note: "one stamp per session (2026-11-18); a global stamp silenced peers",
  },
  "route-ledger-": {
    kind: "append-only",
    note: "route-ledger-YYYYMMDD.jsonl; every row carries sessionId + tuiPid",
  },

  // ── caches: last writer wins, and a lost race costs one refresh ─────────
  "gateway-health.json": {
    kind: "last-writer-wins",
    note: "lane cooldown cache",
  },
  "provider-health.json": {
    kind: "last-writer-wins",
    note: "per-model cooldown cache",
  },
  "caduceus-models.json": {
    kind: "last-writer-wins",
    note: "gateway catalog cache",
  },
  "caduceus-state.json": {
    kind: "last-writer-wins",
    note: "gateway process bookkeeping",
  },
  "prompt-suggestion.json": {
    kind: "last-writer-wins",
    note: "last model suggestion",
  },

  // ── operator settings: one value, the last editor is the intent ─────────
  "agent-mode.json": {
    kind: "last-writer-wins",
    note: "operator mode; a second session overwrites it (accepted, visible)",
  },
  "caduceus-prefs.json": { kind: "user-config", note: "routing prefs panel" },
  "couplings.json": { kind: "user-config", note: "model couplings" },
  "watchdog-decision.json": {
    kind: "user-config",
    note: "decision-lane config",
  },
  "modes.json": { kind: "user-config", note: "agent mode definitions" },
  "permissions-mode": { kind: "user-config", note: "permissions posture" },
  perms: { kind: "user-config", note: "permission rules" },
  "accounts.json": { kind: "user-config", note: "cloud accounts" },
  "routing-profile": { kind: "user-config", note: "routing profile" },
  hooks: { kind: "user-config", note: "harness hook definitions" },
  toolchecks: { kind: "user-config", note: "tool check rules" },
  memory: { kind: "user-config", note: "MEMORY.md / USER.md" },
  env: { kind: "user-config", note: "shared env file for extensions" },
  state: { kind: "user-config", note: "state tables" },

  // ── handshakes: fixed names, so two sessions CAN collide ────────────────
  // Classified LWW rather than session-keyed deliberately: these are one-shot
  // question/answer files, and the risk is a session answering another's prompt.
  // Doing nothing here is a decision, not an oversight — it is listed so a
  // future reader sees it.
  "question.json": {
    kind: "last-writer-wins",
    note: "handshake; two concurrent dialogs would collide (known, accepted)",
  },
  "question-response.json": {
    kind: "last-writer-wins",
    note: "handshake answer",
  },
  "inquire-answer.json": { kind: "last-writer-wins", note: "inquire answer" },

  // ── retired ─────────────────────────────────────────────────────────────
  "routing-registry.json": {
    kind: "last-writer-wins",
    note: "RETIRED pre-shard aggregate; deleted 2026-11-18, shards are the only source",
  },
};

/** Names present in the sources but missing from the table (must be empty). */
export function unclassified(discovered: readonly string[]): string[] {
  return [...new Set(discovered)].filter((n) => !(n in SHARED_STATE)).sort();
}

/** Rows whose kind makes cross-session collision impossible or harmless. */
export function collides(discovered: readonly string[]): string[] {
  return [...new Set(discovered)]
    .filter((n) => {
      const entry = SHARED_STATE[n];
      return !!entry && entry.kind === "last-writer-wins";
    })
    .sort();
}
