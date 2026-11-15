// hang-watchdog judgment rules (2026-11-12). Pure + injectable: the
// watchdog NEVER kills — it observes, escalates, and when output stalls it
// asks an advisory judge about liveliness. Unknown stays unknown; the operator decides.
// PROVENANCE: original Officina watchdog rules and activity observation.

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
  lastOutputRevision: number;
  stalledSince: number | null; // ms epoch when output last grew (null = growing)
  judgments: number;
  lastVerdict: string | null;
  judgedAt?: number;
  lastActivityAt?: number;
  lastNoticeAt?: number;
  generation?: number;
  outputSample?: string;
  tail?: string;
  judging?: boolean;
  surfaced?: boolean;
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

/** Extract the normal Pi content envelope as well as legacy text/array updates. */
export function toolOutputText(partial: unknown): string {
  if (typeof partial === "string") return partial;
  if (Array.isArray(partial)) return partial.map((part) => typeof part === "string" ? part : (part?.text ?? "")).join("\n");
  if (partial && typeof partial === "object" && "content" in partial) {
    const content = (partial as { content?: unknown }).content;
    return Array.isArray(content) ? toolOutputText(content) : typeof content === "string" ? content : "";
  }
  return "";
}

function markProgress(call: WatchedCall, revision: number, now: number): void {
  call.lastOutputRevision = revision;
  call.lastActivityAt = now;
  call.stalledSince = null;
  call.judgments = 0;
  call.judgedAt = undefined;
  call.lastNoticeAt = undefined;
  call.lastVerdict = null;
  call.generation = (call.generation ?? 0) + 1;
}

/** Keep a bounded sample, detecting append growth and changed constant-length tails. */
export function observeToolOutput(call: WatchedCall, partial: unknown, now: number): boolean {
  const text = toolOutputText(partial);
  if (!text) return false;
  const tail = text.slice(-TAIL_CHARS).split("\n").slice(-40).join("\n");
  const sample = JSON.stringify([text.length, tail]);
  if (sample === call.outputSample) return false;
  call.outputSample = sample;
  call.tail = tail;
  markProgress(call, call.lastOutputRevision + 1, now);
  return true;
}

/** Paid judgments are bounded per silent episode; deterministic monitoring continues. */
export function tickCall(call: WatchedCall, outputRevision: number, tail: string, input: TickInput): TickAction {
  if (outputRevision > call.lastOutputRevision) {
    markProgress(call, outputRevision, input.now);
    return { kind: "alive" };
  }
  const elapsedS = Math.floor((input.now - call.startedAt) / 1000);
  if (elapsedS < input.softS) return { kind: "none" };
  call.stalledSince ??= call.lastActivityAt ?? call.startedAt;
  const stalledS = Math.floor((input.now - call.stalledSince) / 1000);
  if (stalledS < input.judgeS) {
    if (stalledS >= input.softS && call.lastVerdict === null) {
      call.lastVerdict = "noticed";
      call.lastNoticeAt = input.now;
      return { kind: "notice", message: `hang-watchdog: "${short(call.command)}" — no output for ${stalledS}s (${elapsedS}s total)` };
    }
    return { kind: "none" };
  }
  if (call.judging) return { kind: "none" };
  const lastCheck = Math.max(call.judgedAt ?? 0, call.lastNoticeAt ?? 0);
  if (lastCheck > 0 && input.now - lastCheck < REJUDGE_EVERY_S * 1000 && call.judgments > 0) return { kind: "none" };
  if (call.judgments >= MAX_JUDGMENTS_PER_CALL) {
    call.lastNoticeAt = input.now;
    return { kind: "notice", message: `hang-watchdog: "${short(call.command)}" — still no output after ${stalledS}s; judge budget exhausted, monitoring continues` };
  }
  call.judgments += 1;
  call.judgedAt = input.now;
  return { kind: "judge", message: `hang-watchdog: judging "${short(call.command)}" (no output ${stalledS}s, ${elapsedS}s total)`, tail };
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
    "Respond with EXACTLY one line: VERDICT: ALIVE or VERDICT: HUNG or VERDICT: UNKNOWN, then one short sentence of evidence.",
    "ALIVE = evidence of this command progressing or legitimately performing finite work. Silence alone proves neither health nor a hang.",
    "UNKNOWN = insufficient evidence. Other sessions or GPU activity do not prove this command is alive.",
    "HUNG = waiting on something that will never finish (stdin prompt, deadlock, repeated identical output).",
  ].join("\n");
}

/** Parse the judge's reply: first VERDICT: line wins; UNKNOWN on
 *  ambiguity. An unavailable or malformed judge is never positive health evidence. */
export function parseVerdict(reply: string): { alive: boolean | null; reason: string } {
  const matches = [...String(reply ?? "").matchAll(/^\s*VERDICT:\s*(ALIVE|HUNG|UNKNOWN)\b([^\n]*)/gim)];
  const m = matches[0];
  if (matches.length !== 1 || /VERDICT:|\bor\s+(ALIVE|HUNG|UNKNOWN)\b/i.test(m?.[2] ?? "")) {
    return { alive: null, reason: "judge reply unparseable — liveness unknown" };
  }
  const rest = String(reply ?? "").slice((m.index ?? 0) + m[0].length - m[2].length).trim();
  const reason = rest.split("\n")[0]?.slice(0, 160) || "(no evidence given)";
  return { alive: m[1].toUpperCase() === "UNKNOWN" ? null : m[1].toUpperCase() === "ALIVE", reason };
}

/** Best free cloud lane for the judgment call (catalog cache facts). */
export function pickJudgeModel(freeIds: string[]): string | null {
  return freeIds.length > 0 ? freeIds[0] : null;
}
