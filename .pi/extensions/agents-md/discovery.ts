// AGENTS.md discovery + digest (2026-11-14, operator request: "make Officina
// reactive to AGENTS.md, or at least aware of it").
//
// pi has no built-in AGENTS.md support; this extension makes the harness
// aware of the operator's project rules without blowing the always-on budget:
// a compact DIGEST (headings + bullets, capped) is injected once per session
// and re-injected when a file changes. The full text stays on disk.
//
// Pure logic lives here (paths in, digest out) so it is unit-tested without a
// pi runtime; the extension only handles I/O + timing.

import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

export interface AgentsFile {
  path: string;
  text: string;
  mtime: number;
}

/** Nearest-first ancestors collected from cwd up to the filesystem root. */
export function findAgentsFiles(
  cwd: string,
  opts: { exists?: (p: string) => boolean; read?: (p: string) => string; stat?: (p: string) => number } = {},
): string[] {
  const exists = opts.exists ?? existsSync;
  const out: string[] = [];
  let dir = resolve(cwd);
  for (let i = 0; i < 32; i += 1) {
    const candidate = join(dir, "AGENTS.md");
    if (exists(candidate)) out.push(candidate);
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  // Nearest last: later entries take precedence in the digest.
  return out.reverse();
}

export function loadAgentsFiles(
  cwd: string,
  env: NodeJS.ProcessEnv = process.env,
): AgentsFile[] {
  const override = String(env.OFFICINA_AGENTS_FILES ?? "")
    .split(":")
    .map((p) => p.trim())
    .filter(Boolean);
  const paths = override.length > 0 ? override : findAgentsFiles(cwd);
  const files: AgentsFile[] = [];
  for (const path of paths) {
    try {
      files.push({
        path,
        text: readFileSync(path, "utf8"),
        mtime: statSync(path).mtimeMs,
      });
    } catch {
      // missing/unreadable — skip
    }
  }
  return files;
}

/** Change signature: paths + mtimes (cheap; called on every turn boundary). */
export function agentsSignature(files: readonly AgentsFile[]): string {
  return files
    .map((f) => `${f.path}@${Math.floor(f.mtime)}`)
    .sort()
    .join("|");
}

export interface DigestOptions {
  maxChars?: number;
  cwd?: string;
}

/**
 * Compact digest: per file, every heading and bullet (the rule-bearing lines),
 * bounded. `truncated` reports that the cap dropped content so the caller can
 * point at the file instead of pretending the digest is complete.
 */
export function buildDigest(
  files: readonly AgentsFile[],
  opts: DigestOptions = {},
): { text: string; rules: number; truncated: boolean } {
  const maxChars = Math.max(400, opts.maxChars ?? 3000);
  const cwd = opts.cwd ? resolve(opts.cwd) : undefined;
  const lines: string[] = [];
  let rules = 0;
  let truncated = false;
  if (files.length > 0) {
    lines.push(
      "## Project rules (AGENTS.md digest — headings and bullets; the full files are on disk)",
    );
  }
  for (const file of files) {
    const shown = cwd ? relative(cwd, file.path) || file.path : file.path;
    const body = file.text.split(/\r?\n/);
    const kept: string[] = [];
    for (const raw of body) {
      const line = raw.trimEnd();
      if (!line.trim()) continue;
      const isHeading = /^#{1,4}\s+/.test(line);
      const isBullet = /^\s*(?:[-*]|\d+[.)])\s+/.test(line);
      if (!isHeading && !isBullet) continue;
      kept.push(line.length > 220 ? `${line.slice(0, 219)}…` : line);
      if (isBullet) rules += 1;
    }
    if (kept.length === 0) continue;
    const header = `\n### ${shown}`;
    if (lines.join("\n").length + header.length > maxChars) {
      truncated = true;
      break;
    }
    lines.push(header);
    for (const line of kept) {
      if (lines.join("\n").length + line.length + 1 > maxChars) {
        truncated = true;
        break;
      }
      lines.push(line);
    }
    if (truncated) break;
  }
  if (truncated) {
    lines.push("… [digest truncated — read the AGENTS.md file(s) for the rest]");
  }
  return { text: lines.join("\n"), rules, truncated };
}

/** One-line status for the operator. */
export function digestSummary(files: readonly AgentsFile[], rules: number, chars: number): string {
  if (files.length === 0) return "AGENTS.md: none found";
  return `AGENTS.md: ${files.length} file(s) · ${rules} rule(s) · digest ${chars} chars`;
}
