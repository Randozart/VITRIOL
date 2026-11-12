// fix-loop rules (exec-infra stage 7, I7). Pure + injectable: the loop is
// explicit, capped, single-flight, day-capped, and auto-commit defaults OFF.

export const MAX_ROUNDS_HARD = 10;
export const MAX_PER_DAY = 6;

export function planRounds(maxArg: string | undefined, env: NodeJS.ProcessEnv = process.env): number {
  if (env.OFFICINA_NO_FIX_LOOP === "1") return 0;
  const raw = String(maxArg ?? "").trim();
  if (raw === "") return 3;
  const n = Number(raw);
  if (!Number.isFinite(n)) return 3;
  return Math.max(1, Math.min(MAX_ROUNDS_HARD, Math.trunc(n)));
}

export interface LoopState {
  version: 1;
  day: string;
  runsToday: number;
}

export function dayKey(now = new Date()): string {
  return now.toISOString().slice(0, 10);
}

/** Day-capped: a fresh day resets the counter; over-cap refuses (I7). */
export function canRun(state: LoopState | null, now = new Date()): { ok: boolean; reason: string } {
  if (!state || state.version !== 1) return { ok: true, reason: "" };
  const today = dayKey(now);
  if (state.day !== today) return { ok: true, reason: "" };
  if (state.runsToday >= MAX_PER_DAY) {
    return { ok: false, reason: `fix-loop: day-capped (${state.runsToday}/${MAX_PER_DAY} runs today) — try tomorrow` };
  }
  return { ok: true, reason: "" };
}

export function bumpRun(state: LoopState | null, now = new Date()): LoopState {
  const today = dayKey(now);
  if (!state || state.day !== today) {
    return { version: 1, day: today, runsToday: 1 };
  }
  return { version: 1, day: today, runsToday: state.runsToday + 1 };
}

export interface FixTarget {
  gateId: string;
  gateLabel: string;
  diag?: { file?: string | null; message?: unknown } | null;
}

/** The fix turn prompt: names the failing gates + their first diagnostics. */
export function fixPrompt(regressions: FixTarget[], round: number, maxRounds: number): string {
  const lines = regressions
    .slice(0, 12)
    .map(
      (r) =>
        `- ${r.gateId} (${r.gateLabel})${r.diag?.file ? ` — ${r.diag.file}` : ""}${
          r.diag?.message ? `: ${String(r.diag.message).slice(0, 200)}` : ""
        }`,
    );
  return [
    `fix-loop round ${round}/${maxRounds}: these gates are failing (new diagnostics, NOT pre-existing):`,
    ...lines,
    "",
    "Fix every failure. Run the gates yourself to confirm before you stop. If a failure is NOT fixable",
    "(flaky, environmental, or a real design conflict), STOP and explain instead of thrashing.",
  ].join("\n");
}

/** Auto-commit (I7): default OFF; only after GREEN; only with changes. */
export function autocommitEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.OFFICINA_FIXLOOP_AUTOCOMMIT === "1";
}

export const AUTOCOMMIT_MESSAGE_PREFIX = "fix-loop: gates green";
