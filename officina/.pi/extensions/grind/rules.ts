// grind rules (2026-11-14) — PURE decision logic for the GRIND loop.
//
// The loop exists because a directive alone cannot continue work: pi ends a
// turn when the model stops calling tools. The extension re-prompts on
// agent_end until the model proves completion (GRIND: DONE) or reports a
// block (GRIND: BLOCKED), within hard caps, and always yields to the user.
//
// Invariants pinned by rules.test.ts:
//   I3 abort never re-prompts; I4 bounded caps; I5 one continuation per
//   settled turn; strict stop marker.

export type Marker =
  | { kind: "done" }
  | { kind: "blocked"; reason: string }
  | { kind: "none" };

export const DEFAULT_MAX_TURNS = 24;
export const HARD_MAX_TURNS = 100;
export const DEFAULT_MAX_MINUTES = 120;

export function grindDisabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.OFFICINA_NO_GRIND === "1";
}

function positiveInt(raw: string | undefined, fallback: number, hard: number): number {
  const n = Number(String(raw ?? "").trim());
  if (!Number.isFinite(n) || n <= 0) return fallback;
  return Math.max(1, Math.min(hard, Math.trunc(n)));
}

export function maxTurns(env: NodeJS.ProcessEnv = process.env): number {
  return positiveInt(env.OFFICINA_GRIND_MAX_TURNS, DEFAULT_MAX_TURNS, HARD_MAX_TURNS);
}

export function maxMinutes(env: NodeJS.ProcessEnv = process.env): number {
  return positiveInt(env.OFFICINA_GRIND_MAX_MINUTES, DEFAULT_MAX_MINUTES, 24 * 60);
}

/**
 * Completion markers are the FINAL non-empty line of the assistant text —
 * a deliberate terminal statement, never an inline mention.
 */
export function parseMarker(text: string): Marker {
  const lines = String(text ?? "")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
  const last = lines[lines.length - 1] ?? "";
  const blocked = /^GRIND:\s*BLOCKED\b[: ]*(.*)$/i.exec(last);
  if (blocked) {
    return { kind: "blocked", reason: (blocked[1] ?? "").trim() || "no reason given" };
  }
  if (/^GRIND:\s*DONE\b/i.test(last)) return { kind: "done" };
  return { kind: "none" };
}

export interface TaskSnapshot {
  pending: number;
  inProgress: number;
  open: string[];
}

export interface GrindState {
  /** Continuation messages sent in this loop. */
  rounds: number;
  startedAt: number;
  /** DONE claims rejected because tasks/verification still say otherwise. */
  doneReminders: number;
  /** Signature of the last settle, for the no-progress guard. */
  lastSignature?: string;
  /** Consecutive settles with the same signature. */
  flatRounds: number;
}

export function freshState(now: number): GrindState {
  return { rounds: 0, startedAt: now, doneReminders: 0, flatRounds: 0 };
}

export type StopReason =
  | "mode-changed"
  | "disabled"
  | "aborted"
  | "user-input"
  | "queued"
  | "compacting"
  | "will-retry"
  | "done"
  | "blocked"
  | "cap-turns"
  | "cap-minutes"
  | "no-progress";

export interface DecideInput {
  mode: string | null;
  aborted: boolean;
  userDisarmed: boolean;
  queued: boolean;
  compacting: boolean;
  willRetry: boolean;
  assistantText: string;
  tasks: TaskSnapshot | null;
  /** null = verification not run this round. */
  verify: { ran: boolean; failed: string[] } | null;
  state: GrindState;
  now: number;
  maxTurns: number;
  maxMinutes: number;
  /** Changes when the turn actually did something (task list, text). */
  signature: string;
}

export interface ContinueDecision {
  action: "continue";
  round: number;
  prompt: string;
  next: GrindState;
}

export interface StopDecision {
  action: "stop";
  reason: StopReason;
  note: string;
  next: GrindState | null;
}

export type Decision = ContinueDecision | StopDecision;

/** Pending + in-progress tasks keep the loop alive. */
export function hasOpenWork(tasks: TaskSnapshot | null): boolean {
  return !!tasks && tasks.pending + tasks.inProgress > 0;
}

export function decide(input: DecideInput): Decision {
  const { state } = input;
  const stop = (reason: StopReason, note: string): StopDecision => ({
    action: "stop",
    reason,
    note,
    next: null,
  });

  if (input.mode !== "grind") return stop("mode-changed", "mode is not grind");

  const marker = parseMarker(input.assistantText);

  if (input.aborted) return stop("aborted", "turn aborted (Esc) — loop stands down");
  if (input.userDisarmed) return stop("user-input", "user took the wheel");
  if (input.queued) return stop("queued", "a steering/follow-up message is queued");
  if (input.compacting) return stop("compacting", "compaction in flight");
  if (input.willRetry) return stop("will-retry", "pi is auto-retrying this turn");

  if (marker.kind === "blocked") {
    return stop("blocked", `blocked: ${marker.reason}`);
  }

  if (marker.kind === "done") {
    const verified = !input.verify || input.verify.failed.length === 0;
    if (!hasOpenWork(input.tasks) && verified) {
      return stop("done", "GRIND: DONE — task list empty, changes verified");
    }
    // One honest reminder; then accept the model's judgement rather than
    // fighting it (a stuck reminder loop is worse than a premature stop).
    if (state.doneReminders >= 1) {
      return stop(
        "done",
        "GRIND: DONE accepted after reminder (task list or gates still open)",
      );
    }
    const next = { ...state, doneReminders: state.doneReminders + 1, rounds: state.rounds + 1 };
    const why = hasOpenWork(input.tasks)
      ? `${input.tasks!.pending + input.tasks!.inProgress} task(s) still open`
      : "verification gates are failing";
    return {
      action: "continue",
      round: next.rounds,
      prompt: continuationPrompt(next.rounds, input.maxTurns, input.tasks, input.verify, {
        doneRejected: why,
      }),
      next,
    };
  }

  if (state.rounds >= input.maxTurns) {
    return stop(
      "cap-turns",
      `turn cap reached (${input.maxTurns}) without GRIND: DONE`,
    );
  }
  if (input.now - state.startedAt > input.maxMinutes * 60_000) {
    return stop(
      "cap-minutes",
      `time cap reached (${input.maxMinutes} min) without GRIND: DONE`,
    );
  }

  const flat = !!input.tasks && input.signature === state.lastSignature;
  const flatRounds = flat ? state.flatRounds + 1 : 0;
  if (flatRounds >= 3) {
    return stop(
      "no-progress",
      "three settles with no change — stopping instead of thrashing",
    );
  }

  const next: GrindState = {
    ...state,
    rounds: state.rounds + 1,
    lastSignature: input.signature,
    flatRounds,
  };
  return {
    action: "continue",
    round: next.rounds,
    prompt: continuationPrompt(next.rounds, input.maxTurns, input.tasks, input.verify, {}),
    next,
  };
}

export interface PromptOptions {
  doneRejected?: string;
}

export function continuationPrompt(
  round: number,
  max: number,
  tasks: TaskSnapshot | null,
  verify: { ran: boolean; failed: string[] } | null,
  opts: PromptOptions = {},
): string {
  const lines: string[] = [];
  if (opts.doneRejected) {
    lines.push(
      `GRIND round ${round}/${max}: you claimed GRIND: DONE, but ${opts.doneRejected}.`,
      "Continue and finish the remaining work, then end with GRIND: DONE.",
    );
  } else {
    lines.push(
      `GRIND round ${round}/${max}: the turn ended without GRIND: DONE or GRIND: BLOCKED.`,
      "Continue the work — do not re-plan from scratch and do not restate what you already did.",
    );
  }
  if (tasks && tasks.open.length > 0) {
    lines.push("", "Open tasks:");
    for (const t of tasks.open.slice(0, 10)) lines.push(`- ${t}`);
    if (tasks.open.length > 10) lines.push(`- … ${tasks.open.length - 10} more`);
  }
  if (verify && verify.failed.length > 0) {
    lines.push("", "Failing verification gates:");
    for (const g of verify.failed.slice(0, 8)) lines.push(`- ${g}`);
    lines.push("Fix these before continuing.");
  }
  lines.push(
    "",
    "Rules: keep going until the task list is empty and changes are verified; then finish with a final line `GRIND: DONE`. If progress is impossible without the user, finish with `GRIND: BLOCKED: <reason>`.",
  );
  return lines.join("\n");
}

/** Cheap change signature: the external task list is the only progress
 *  signal the loop can observe without diffing the repo. */
export function activitySignature(tasks: TaskSnapshot | null): string {
  if (!tasks) return "no-tasks";
  let h = 0x811c9dc5;
  const feed = `${tasks.pending}|${tasks.inProgress}|${tasks.open.join("~")}`;
  for (let i = 0; i < feed.length; i++) {
    h ^= feed.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return (h >>> 0).toString(16);
}
