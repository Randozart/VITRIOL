// Cross-session file claims (2026-11-18).
//
// Several Officina sessions can run in one project, and until now two of them
// could edit the same file with no signal at all — the only backstop was git's
// index.lock at commit time. This publishes "session P has file F open" as a
// per-pid shard and lets a session SEE that a file is also being touched
// elsewhere.
//
// Deliberately advisory (invariant I1): a claim never blocks a write and never
// fails a tool call. A claim read failure degrades to silence — losing a warning
// must never cost the operator their edit.
//
// Same shape as the routing registry (2026-11-11): one file per pid, last-writer
// -wins within a pid, TTL-pruned on read, garbage tolerated. Two writers can
// never lose each other's claims because they touch different files.

import {
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

/** How long a claim is believed. A session that goes quiet stops claiming. */
export const CLAIM_TTL_MS = 10 * 60_000;

export interface ClaimEntry {
  pid: number;
  sessionId: string;
  paths: string[];
  at: number;
}

/** Directory of claim shards (`OFFICINA_CLAIMS_DIR` overrides for tests). */
export function claimsDir(env: NodeJS.ProcessEnv = process.env): string {
  return (
    env.OFFICINA_CLAIMS_DIR || join(homedir(), ".vitriol", "officina", "claims")
  );
}

/** Absolute-or-relative path, normalised so two spellings compare equal. */
export function normalizePath(path: string): string {
  return path.trim().replace(/^\.\//, "").replace(/\/+$/, "");
}

function writeAtomic(path: string, body: string): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, body, { mode: 0o600 });
  renameSync(tmp, path);
}

/** Publish (replace) this process's claims. Best effort. */
export function publishClaims(
  entry: { pid: number; sessionId: string; paths: string[] },
  dir: string = claimsDir(),
  now: number = Date.now(),
): void {
  try {
    const paths = [...new Set(entry.paths.map(normalizePath))].filter(Boolean);
    if (paths.length === 0) {
      // Nothing open: remove the shard rather than claiming an empty set.
      writeAtomic(
        join(dir, `${entry.pid}.json`),
        JSON.stringify({ paths: [] }),
      );
      return;
    }
    writeAtomic(
      join(dir, `${entry.pid}.json`),
      JSON.stringify({
        pid: entry.pid,
        sessionId: entry.sessionId,
        paths,
        at: now,
      }),
    );
  } catch {
    // advisory only
  }
}

/** Every live claim from every session (including this one). */
export function readClaims(
  now: number = Date.now(),
  ttlMs: number = CLAIM_TTL_MS,
  dir: string = claimsDir(),
): ClaimEntry[] {
  let files: string[] = [];
  try {
    files = readdirSync(dir).filter((f) => f.endsWith(".json"));
  } catch {
    return [];
  }
  const out: ClaimEntry[] = [];
  for (const file of files) {
    try {
      const doc = JSON.parse(
        readFileSync(join(dir, file), "utf8"),
      ) as Partial<ClaimEntry>;
      if (typeof doc.pid !== "number" || !Array.isArray(doc.paths)) continue;
      if (typeof doc.at === "number" && now - doc.at > ttlMs) continue;
      out.push({
        pid: doc.pid,
        sessionId: doc.sessionId ?? `pid-${doc.pid}`,
        paths: doc.paths.map(normalizePath).filter(Boolean),
        at: doc.at ?? now,
      });
    } catch {
      // a torn shard costs one warning, never a tool call
    }
  }
  return out;
}

/** Another live session's claim on any of `paths` (self excluded). */
export function foreignClaims(
  paths: readonly string[],
  selfPid: number,
  now: number = Date.now(),
  ttlMs: number = CLAIM_TTL_MS,
  dir: string = claimsDir(),
): { path: string; pid: number }[] {
  const mine = new Set(paths.map(normalizePath));
  const out: { path: string; pid: number }[] = [];
  for (const claim of readClaims(now, ttlMs, dir)) {
    if (claim.pid === selfPid) continue;
    for (const path of claim.paths) {
      if (mine.has(path)) out.push({ path, pid: claim.pid });
    }
  }
  return out;
}

/**
 * One advisory line for the band, or null when nothing collides.
 *
 * Names the OTHER session by pid — the operator can find it in the session list
 * — and never blocks anything.
 */
export function claimNote(
  conflicts: readonly { path: string; pid: number }[],
): string | null {
  if (conflicts.length === 0) return null;
  const [first] = conflicts;
  const base = first.path.split("/").pop() ?? first.path;
  const more = conflicts.length > 1 ? ` (+${conflicts.length - 1})` : "";
  return `⚠ ${base}${more} also open in session ${first.pid}`;
}
