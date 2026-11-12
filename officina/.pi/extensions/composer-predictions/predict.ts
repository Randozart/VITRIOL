// Composer prediction rules (codex-qol stage 8). Pure + injectable: the
// extension turns one advisory line per finished run; I7 — it never sends.

import { readFileSync } from "node:fs";

export interface VerifyLast {
  version: number;
  ts: number;
  tier: string;
  ok: boolean;
  regressions: string[];
  in_progress: string[];
}

/** Advisory next-step line, or null for silence. */
export function predict(verify: VerifyLast | null, dirtyCount: number, now: number): string | null {
  if (verify && typeof verify.ok === "boolean" && !verify.ok) {
    const n = (verify.regressions?.length ?? 0) + (verify.in_progress?.length ?? 0);
    const label = n > 0 ? `${n} gate${n === 1 ? "" : "s"} failing` : "gates failing";
    const rerun = verify.tier === "cheap" ? " (/verify full)" : " (/verify)";
    return `▸ next: fix the failing gates — ${label}${rerun}`;
  }
  if (dirtyCount > 0) {
    return `▸ next: commit the work (${dirtyCount} changed file${dirtyCount === 1 ? "" : "s"})`;
  }
  return null;
}

/** Tolerant reader: missing/corrupt → null (I7). */
export function readVerifyLast(
  dir: string,
  read: (path: string) => string = (path) => readFileSync(path, "utf8"),
): VerifyLast | null {
  try {
    const doc = JSON.parse(read(`${dir}/verify-last.json`));
    if (!doc || typeof doc !== "object") return null;
    return doc as VerifyLast;
  } catch {
    return null;
  }
}

/** `git status --porcelain` line count; -1 = unknown (git missing etc.). */
export function dirtyCount(run: (args: string[]) => string): number {
  const out = run(["status", "--porcelain"]);
  if (out === "") return -1;
  return out.split("\n").filter((l) => l.trim().length > 0).length;
}
