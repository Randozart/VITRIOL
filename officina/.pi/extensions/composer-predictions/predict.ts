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

/** Task text as a one-line prompt step; never a paragraph. */
export function clipTask(text: string, max = 120): string {
  const one = text
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[.;:,]+$/, "");
  return one.length > max ? `${one.slice(0, max - 1).trimEnd()}…` : one;
}

/** Short process annotation that may DECORATE a task step, never replace it. */
function hygiene(verify: VerifyLast | null, dirtyCount: number): string[] {
  const hints: string[] = [];
  if (verify && typeof verify.ok === "boolean" && !verify.ok) {
    const n =
      (verify.regressions?.length ?? 0) + (verify.in_progress?.length ?? 0);
    hints.push(
      n > 0 ? `${n} gate${n === 1 ? "" : "s"} failing` : "gates failing",
    );
  }
  if (dirtyCount > 0) {
    hints.push(
      `${dirtyCount} changed file${dirtyCount === 1 ? "" : "s"} to commit`,
    );
  }
  return hints;
}

/**
 * Advisory next-step line, or null for silence.
 *
 * The recommendation follows the WORK, not the rulebook (2026-11-16, operator
 * report: "the recommended next text is spilling over the LLM instructions").
 * A task in progress becomes the step; gate/commit state may only annotate it.
 * With no task there is no request-aligned step to offer, so the heuristic stays
 * silent and the LLM draft (which sees the operator's last request) fills the
 * space — or nothing does.
 */
export function predict(
  verify: VerifyLast | null,
  dirtyCount: number,
  now: number,
  task: string | null = null,
): string | null {
  void now; // kept for the existing call shape; the rules are stateless
  const step = (task ?? "").trim();
  if (!step) return null; // D4: silence beats process noise
  const line = `▸ next: ${clipTask(step)}`;
  const hints = hygiene(verify, dirtyCount);
  return hints.length ? `${line} · ${hints.join(" · ")}` : line;
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
