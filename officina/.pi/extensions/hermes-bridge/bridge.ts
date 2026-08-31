// hermes-bridge — READ-ONLY scaffold access to Hermes memory (REPORT-02
// step 16, PLAN.md §3c: "sub-coders can query Hermes memory via the
// hermes-bridge extension (read-only)").
//
// This is the ONE sanctioned channel from scaffold to gateway; the contract
// is read-only (writes belong to Hermes' memory tool + the step-18 extractor,
// so the gateway remains the sole author of memory — Rule 2).
//
// Two surfaces, both verified against real stores 2026-08-29:
//   1. FTS5 session search: ~/.hermes/state.db messages_fts JOIN messages
//      JOIN sessions, ORDER BY rank (bm25), snippet-capped
//   2. persistent facts: ~/.hermes/memories/MEMORY.md + USER.md (head-capped)
//
// node:sqlite is experimental in Node 22 — used behind try/catch; absence of
// the module degrades the tool to a clear error, never a crash at load.
// Kill switch: TRIS_NO_HERMES_BRIDGE=1.

import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { join } from "node:path";

const require_ = createRequire(import.meta.url);

/** Load node:sqlite through CJS — bundler-safe against the ESM resolver. */
type SqliteModule = { DatabaseSync: new (p: string, o?: { readOnly?: boolean }) => {
  prepare: (sql: string) => { all: (...args: unknown[]) => unknown[]; run: (...args: unknown[]) => unknown };
  close: () => void;
} };

export function loadSqlite(): SqliteModule {
  return require_("node:sqlite") as SqliteModule;
}

export interface BridgeConfig {
  enabled: boolean;
  stateDb: string;
  memoriesDir: string;
  maxRows: number; // result rows per search
  snippetChars: number; // per-row content cap
  memoryHeadLines: number; // MEMORY.md lines returned
}

export function bridgeConfig(env: NodeJS.ProcessEnv = process.env): BridgeConfig {
  const home = env.HOME || homedir();
  return {
    enabled: env.TRIS_NO_HERMES_BRIDGE !== "1",
    stateDb: env.TRIS_HERMES_DB || join(home, ".hermes/state.db"),
    memoriesDir: env.TRIS_HERMES_MEM || join(home, ".hermes/memories"),
    maxRows: 5,
    snippetChars: 300,
    memoryHeadLines: 50,
  };
}

/** Build the FTS5 search SQL. Exported so the shape is testable without sqlite. */
export function searchSql(): string {
  return `SELECT m.id, s.id AS session_id, m.role, m.timestamp,
         substr(replace(m.content, char(10), ' '), 1, ?) AS snippet
    FROM messages_fts f
    JOIN messages m ON m.id = f.rowid
    JOIN sessions s ON s.id = m.session_id
   WHERE messages_fts MATCH ?
   ORDER BY rank
   LIMIT ?`;
}

/** Escape bare quotes a naive user query could turn into FTS syntax errors. */
export function sanitizeFtsQuery(q: string): string {
  const trimmed = q.trim().slice(0, 200);
  return trimmed.replace(/["']/g, " ").replace(/\s+/g, " ").trim();
}

/** Head-cap a memory file read; returns "" for missing files. */
export function headLines(text: string, n: number): string {
  return text.split("\n").slice(0, n).join("\n");
}

/** Read MEMORY.md / USER.md (whichever exist), capped. */
export function readPersistentFacts(dir: string, maxLines: number): string {
  const parts: string[] = [];
  for (const f of ["MEMORY.md", "USER.md"]) {
    const p = join(dir, f);
    if (!existsSync(p)) continue;
    try {
      parts.push(`### ${f}\n${headLines(readFileSync(p, "utf8"), maxLines)}`);
    } catch {
      // unreadable store is absence of knowledge, not an error
    }
  }
  return parts.join("\n\n");
}

/** Format search rows as compact lines for the model (no raw JSON noise). */
export function formatHits(rows: Array<Record<string, unknown>>, cfg: BridgeConfig): string {
  if (rows.length === 0) return "hermes memory: no matches.";
  return rows
    .map((r) => {
      const ts = Number(r.timestamp ?? 0);
      const when = ts ? new Date(ts * 1000).toISOString().slice(0, 10) : "?";
      return `• [${when} ${String(r.role ?? "?")}] session ${String(r.session_id ?? "?").slice(0, 18)}: ${String(r.snippet ?? "").slice(0, cfg.snippetChars)}`;
    })
    .join("\n");
}

/** One-shot FTS5 query; null = store unavailable (caller renders the reason). */
export async function runSearch(dbPath: string, query: string, cfg: BridgeConfig): Promise<string | null> {
  if (!existsSync(dbPath)) return null;
  const q = sanitizeFtsQuery(query);
  if (!q) return "hermes memory: empty query.";
  try {
    const { DatabaseSync } = loadSqlite();
    const db = new DatabaseSync(dbPath, { readOnly: true });
    try {
      const rows = db.prepare(searchSql()).all(cfg.snippetChars, q, cfg.maxRows) as Array<Record<string, unknown>>;
      return formatHits(rows, cfg);
    } finally {
      db.close();
    }
  } catch (e) {
    return `hermes memory search failed: ${(e as Error).message.slice(0, 200)}`;
  }
}
