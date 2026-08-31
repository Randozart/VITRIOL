import { appendFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// Harness events — fire-and-forget JSONL for the tris cockpit (Round 4 T1,
// schema: trismegistus/docs/TRIS-EXPERIENCE.md). One line per pipeline-stage
// firing so BUDGET/PIPELINE panes show what actually ran and what it freed.
//
// Contract (same discipline as tris_lib.emit_event): NEVER throws, NEVER
// blocks the producing turn meaningfully (sync append to a tiny local file
// is microseconds; failure is swallowed whole). Lives inside the extension,
// so the stage's own kill switch already gates it (Rule 15: switch off =
// silence, no separate events kill switch needed).

export type EventSource =
  | "lc-clearer"
  | "lc-rtk"
  | "lc-ckpt"
  | "lc-relay"
  | "lc-tasks"
  | "lc-perms"
  | "lc-lane"
  | "lc-format"
  | "lc-churn"
  | "lc-fidelity"
  | "lc-contract"
  | "lc-imports"
  | "lc-ledger"
  | "lc-bg";

export interface HarnessEvent {
  ts: number;
  src: EventSource;
  ev: string;
  detail?: string;
  freed_tokens?: number;
  turn?: number;
  session?: string;
}

// SS4 (2026-08-31): state consolidated under ~/.vitriol/officina/state.
// One-shot migration: a legacy ~/.local/state/trismegistus store is moved
// (not copied) on first use, and a marker symlink keeps old readers valid.
export function eventsPath(env: NodeJS.ProcessEnv = process.env): string {
  const dir = env.TRIS_STATE_DIR || join(env.HOME || homedir(), ".vitriol", "officina", "state");
  return join(dir, "events.jsonl");
}

/** Build an event record (pure; testable). */
export function harnessEvent(src: EventSource, ev: string, fields: Partial<HarnessEvent> = {}): HarnessEvent {
  const rec: HarnessEvent = { ts: Date.now() / 1000, src, ev };
  if (fields.detail !== undefined) rec.detail = String(fields.detail).slice(0, 200);
  if (fields.freed_tokens !== undefined) rec.freed_tokens = Math.max(0, Math.floor(fields.freed_tokens));
  if (fields.turn !== undefined) rec.turn = Math.floor(fields.turn);
  if (fields.session) rec.session = fields.session.replace(/[^a-zA-Z0-9._-]/g, "_").slice(0, 80);
  return rec;
}

/** Append one event; swallows every failure (observer must never bite). */
export function emitHarnessEvent(rec: HarnessEvent): void {
  try {
    const p = eventsPath();
    mkdirSync(p.slice(0, p.lastIndexOf("/")), { recursive: true });
    appendFileSync(p, JSON.stringify(rec) + "\n");
  } catch {
    // observability is best-effort by contract
  }
}
