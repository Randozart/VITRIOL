// toolchecks — per-tool-call check outcomes as a FACTS file (2026-11-14).
//
// format-gate / diagnostics-loop / praetor run after an edit/write and the
// TUI shows their outcome on the originating tool block. Extensions cannot
// pass data to the Rust TUI through pi's RPC surface, so the established
// facts-file contract is used: ~/.vitriol/officina/toolchecks/<toolCallId>.json
// (atomic merge, pruned to recent files).
//
// Content is status only — never source text (privacy + size).

import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export interface ToolCheckDoc {
  version: 1;
  ts: number;
  format?: { tool: string; changed: boolean };
  syntax?: { ok: boolean; output?: string };
  praetor?: { count: number };
}

export function toolCheckDir(env: NodeJS.ProcessEnv = process.env): string {
  return join(env.HOME || homedir(), ".vitriol", "officina", "toolchecks");
}

export function toolCheckPath(toolCallId: string, env: NodeJS.ProcessEnv = process.env): string {
  const safe = String(toolCallId).replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 120);
  return join(toolCheckDir(env), `${safe || "unknown"}.json`);
}

/** Keep the directory bounded: drop files older than a day, cap at 500. */
function prune(dir: string): void {
  try {
    const names = readdirSync(dir).filter((n) => n.endsWith(".json"));
    const now = Date.now();
    const kept: Array<{ path: string; mtime: number }> = [];
    for (const n of names) {
      const path = join(dir, n);
      try {
        const mtime = statSync(path).mtimeMs;
        if (now - mtime > 24 * 60 * 60 * 1000) {
          unlinkSync(path);
        } else {
          kept.push({ path, mtime });
        }
      } catch {
        // vanished mid-scan
      }
    }
    if (kept.length > 500) {
      kept.sort((a, b) => a.mtime - b.mtime);
      for (const old of kept.slice(0, kept.length - 500)) {
        try {
          unlinkSync(old.path);
        } catch {
          // best effort
        }
      }
    }
  } catch {
    // decoration contract
  }
}

/** Merge one observation into the call's facts file (atomic write). */
export function recordToolCheck(
  toolCallId: string | undefined,
  patch: Partial<ToolCheckDoc>,
  env: NodeJS.ProcessEnv = process.env,
): void {
  if (!toolCallId) return;
  const path = toolCheckPath(toolCallId, env);
  try {
    let doc: ToolCheckDoc = { version: 1, ts: Date.now() };
    if (existsSync(path)) {
      try {
        const parsed = JSON.parse(readFileSync(path, "utf8")) as ToolCheckDoc;
        if (parsed && parsed.version === 1) doc = parsed;
      } catch {
        // corrupt → start fresh
      }
    }
    doc = { ...doc, ...patch, version: 1, ts: Date.now() };
    mkdirSync(toolCheckDir(env), { recursive: true, mode: 0o700 });
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(doc)}\n`, { mode: 0o600 });
    renameSync(tmp, path);
    prune(toolCheckDir(env));
  } catch {
    // decoration contract
  }
}

/** Read a call's facts (the TUI has its own Rust reader). */
export function readToolCheck(
  toolCallId: string,
  env: NodeJS.ProcessEnv = process.env,
): ToolCheckDoc | null {
  try {
    const parsed = JSON.parse(readFileSync(toolCheckPath(toolCallId, env), "utf8")) as ToolCheckDoc;
    return parsed && parsed.version === 1 ? parsed : null;
  } catch {
    return null;
  }
}
