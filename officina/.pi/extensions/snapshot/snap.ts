// snapshot — pure pieces of the per-turn git-snapshot plumbing (OpenCode
// §3.2 / REPORT-02 step 10). The sequence commits the worktree state to
// refs/trismegistus/turns/<n> using a THROWAWAY GIT_INDEX_FILE, so the user's
// branch, index and worktree are never touched. Rewind (§3.2 pairing with
// VITRIOL slot restore §2.2) is an explicit op in index.ts — never automatic.
//
// Default OFF: TRIS_SNAPSHOT=1 arms it (Rule 15 — same flag is the kill
// switch; no surprise git side effects in a workspace).

import type { ExecFileSyncOptions } from "node:child_process";

export interface SnapshotConfig {
  enabled: boolean;
  refPrefix: string;
}

export function snapshotConfig(env: NodeJS.ProcessEnv = process.env): SnapshotConfig {
  return {
    enabled: env.TRIS_SNAPSHOT === "1", // opt-in
    refPrefix: env.TRIS_SNAPSHOT_PREFIX || "refs/trismegistus/turns",
  };
}

/** One git step. "__VAR__" tokens are substituted from captured outputs. */
export interface GitStep {
  argv: string[];
  useTmpIndex: boolean;
  capture?: "tree" | "sha"; // when set, stdout lands in vars[name]
}

/** The full plumbing sequence for one turn (tree/sha chained via placeholders). */
export function snapshotSequence(refPrefix: string, turn: number): GitStep[] {
  return [
    { argv: ["read-tree", "HEAD"], useTmpIndex: true },
    { argv: ["add", "-A", "--"], useTmpIndex: true },
    { argv: ["write-tree"], useTmpIndex: true, capture: "tree" },
    { argv: ["commit-tree", "__tree__", "-p", "HEAD", "-m", `trismegistus snapshot turn ${turn}`], useTmpIndex: false, capture: "sha" },
    { argv: ["update-ref", turnRef(refPrefix, turn), "__sha__"], useTmpIndex: false },
  ];
}

/** Substitute captured vars into a step's argv. Pure. */
export function substituteStep(step: GitStep, vars: Record<string, string>): string[] {
  return step.argv.map((a) => a.replace(/__([a-z]+)__/g, (_, k: string) => vars[k] ?? a));
}

/** Refname for one turn snapshot. */
export function turnRef(prefix: string, turn: number): string {
  return `${prefix}/${turn}`;
}

/** exec options: repo cwd + optional throwaway index env. Pure. */
export function gitOpts(cwd: string, tmpIndex?: string, env: NodeJS.ProcessEnv = process.env): ExecFileSyncOptions {
  return {
    cwd,
    env: tmpIndex ? { ...env, GIT_INDEX_FILE: tmpIndex } : env,
    maxBuffer: 8 * 1024 * 1024,
  } as ExecFileSyncOptions;
}

/** `git status --porcelain` → dirty when any line exists. Pure. */
export function worktreeDirty(porcelain: string): boolean {
  return porcelain.trim().length > 0;
}

/** Injectable git exec: returns trimmed stdout; throws on non-zero. */
export type GitExec = (argv: string[], opts: ExecFileSyncOptions) => string;

/**
 * Run the snapshot sequence for one turn. `tmpIndex` is the pre-created
 * throwaway index path (caller manages its lifecycle). Returns the snapshot
 * commit sha or null when any step fails.
 */
export function runSnapshotSequence(
  repo: string,
  turn: number,
  cfg: SnapshotConfig,
  tmpIndex: string,
  exec: GitExec,
  env: NodeJS.ProcessEnv = process.env,
): string | null {
  const vars: Record<string, string> = {};
  try {
    for (const step of snapshotSequence(cfg.refPrefix, turn)) {
      const out = exec(substituteStep(step, vars), gitOpts(repo, step.useTmpIndex ? tmpIndex : undefined, env));
      if (step.capture) vars[step.capture] = out;
    }
    return vars.sha ?? null;
  } catch {
    return null; // git trouble must never break the coding loop
  }
}
