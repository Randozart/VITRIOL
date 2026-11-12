// hang-watchdog judgment rules (2026-11-12). Pure + injectable: the
// watchdog NEVER kills — it observes, escalates, and when output stalls it
// asks a free cloud lane to judge liveliness. The operator decides.

export const SOFT_S_DEFAULT = 60; // first "no output" notice
export const JUDGE_S_DEFAULT = 180; // first LLM liveliness judgment
export const REJUDGE_EVERY_S = 300; // re-judge cadence while still stalled
export const MAX_JUDGMENTS_PER_CALL = 3;
export const TICK_S = 30;
export const TAIL_CHARS = 1_500;

export interface WatchedCall {
  toolCallId: string;
  command: string;
  startedAt: number;
  lastOutputChars: number;
  stalledSince: number | null; // ms epoch when output last grew (null = growing)
  judgments: number;
  lastVerdict: string | null;
  judgedAt?: number;
}

export interface TickInput {
  now: number;
  softS: number;
  judgeS: number;
}

export type TickAction =
  | { kind: "none" }
  | { kind: "alive" }
  | { kind: "notice"; message: string }
  | { kind: "judge"; message: string; tail: string };

/** One watchdog tick for one watched call. Returns what should surface.
 *  tail is the output text when a judgment is requested. */
export function tickCall(
  call: WatchedCall,
  outputChars: number,
  tail: string,
  input: TickInput,
): TickAction {
  const elapsedS = Math.floor((input.now - call.startedAt) / 1000);
  if (elapsedS < input.softS) return { kind: "none" };
  if (outputChars > call.lastOutputChars) {
    call.lastOutputChars = outputChars;
    call.stalledSince = null;
    return { kind: "alive" };
  }
  if (call.stalledSince === null) {
    // First silent tick. Output that NEVER grew has been stalled since the
    // start; output that stopped began stalling when we last saw it grow.
    call.stalledSince = call.lastOutputChars > 0 ? input.now : call.startedAt;
  }
  const stalledS = Math.floor((input.now - call.stalledSince) / 1000);
  if (stalledS < input.judgeS) {
    // One notice shortly after output stops; judgment comes later.
    if (call.judgments === 0 && call.lastVerdict === null && stalledS >= input.softS) {
      call.lastVerdict = "noticed";
      return {
        kind: "notice",
        message: `hang-watchdog: "${short(call.command)}" — no output for ${stalledS}s (${elapsedS}s total)`,
      };
    }
    return { kind: "none" };
  }
  if (call.judgments >= MAX_JUDGMENTS_PER_CALL) {
    return { kind: "none" };
  }
  if (
    call.lastVerdict !== null &&
    call.lastVerdict !== "noticed" &&
    input.now - (call.judgedAt ?? 0) < REJUDGE_EVERY_S * 1000
  ) {
    return { kind: "none" };
  }
  call.judgments += 1;
  call.judgedAt = input.now;
  return {
    kind: "judge",
    message: `hang-watchdog: judging "${short(call.command)}" (no output ${stalledS}s, ${elapsedS}s total)`,
    tail,
  };
}

export function short(command: string, max = 60): string {
  const one = command.replace(/\s+/g, " ").trim();
  return one.length > max ? `${one.slice(0, max)}…` : one;
}

/** The judgment prompt: tiny, structured, one-word verdict first. */
export function buildJudgePrompt(command: string, elapsedS: number, stalledS: number, tail: string): string {
  const shown = tail.trim() === "" ? "(no output at all — not even a newline)" : tail.slice(-TAIL_CHARS);
  return [
    "You are a build-command liveliness judge. Decide whether a long-running shell command is ALIVE or HUNG.",
    `Command: ${command.slice(0, 300)}`,
    `Running for ${elapsedS}s; no new output for ${stalledS}s.`,
    "Output tail (last 1500 chars):",
    "```",
    shown,
    "```",
    "Respond with EXACTLY one line: VERDICT: ALIVE or VERDICT: HUNG, then one short sentence of evidence.",
    "ALIVE = compilers/builders/solvers legitimately silent mid-work (compiling, linking, solving, downloading).",
    "HUNG = waiting on something that will never finish (stdin prompt, deadlock, repeated identical output).",
  ].join("\n");
}

/** Parse the judge's reply: first VERDICT: line wins; default ALIVE on
 *  ambiguity (fail-open — we never kill on a parse failure). */
export function parseVerdict(reply: string): { alive: boolean; reason: string } {
  const m = /VERDICT:\s*(ALIVE|HUNG)\b/i.exec(String(reply ?? ""));
  if (!m) return { alive: true, reason: "judge reply unparseable — assuming alive" };
  const rest = String(reply ?? "").slice(m.index + m[0].length).trim();
  const reason = rest.split("\n")[0]?.slice(0, 160) || "(no evidence given)";
  return { alive: m[1].toUpperCase() === "ALIVE", reason };
}

/** Best free cloud lane for the judgment call (catalog cache facts). */
export function pickJudgeModel(freeIds: string[]): string | null {
  return freeIds.length > 0 ? freeIds[0] : null;
}
