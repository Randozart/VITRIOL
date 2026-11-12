// Voice suggestions (voice-suggestions stage 1, 2026-11-12). Pure +
// injectable: after a turn ends, generate the likely next prompt in the
// operator's voice and stage it for the TUI's ghost text. Heuristic
// priority (I3): the verify/dirty rules always outrank an LLM draft.

import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export interface SuggestionDoc {
  version: 1;
  id: string;
  text: string;
  source: "llm" | "heuristic";
  created: string;
  expires: number; // ms epoch
}

export const SUGGESTION_TTL_MS = 10 * 60_000;
export const MAX_DRAFT_CHARS = 240;

export function makeSuggestionId(now = Date.now()): string {
  return `sug-${now}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Heuristic result is prompt-shaped already ("fix the failing gates — …").
 *  I3: it wins whenever it fires; the LLM draft only fills silence. */
export function resolveSuggestion(
  heuristic: string | null,
  llmDraft: string | null,
): { text: string; source: "llm" | "heuristic" } | null {
  if (heuristic && heuristic.trim()) return { text: heuristic.trim(), source: "heuristic" };
  const draft = cleanDraft(llmDraft);
  if (draft) return { text: draft, source: "llm" };
  return null;
}

/** Strip fences/quotes/prefixes and clamp — the judge model loves wrapping. */
export function cleanDraft(reply: string | null): string | null {
  if (!reply) return null;
  let text = String(reply).trim();
  const fence = /```(?:\w+)?\n([\s\S]*?)```/.exec(text);
  if (fence) text = fence[1].trim();
  // drop a "SUGGESTION:" style prefix if present
  text = text.replace(/^suggestion\s*:\s*/i, "");
  // first non-empty line only
  text = text.split("\n").map((l) => l.trim()).filter(Boolean)[0] ?? "";
  // unwrap a single layer of quotes
  text = text.replace(/^["'«]+|["'»]+$/g, "").trim();
  if (!text) return null;
  if (text.length > MAX_DRAFT_CHARS) text = `${text.slice(0, MAX_DRAFT_CHARS - 1).trimEnd()}…`;
  return text;
}

/** Render the voice profile into the generation prompt (absent → generic). */
export function voiceBlock(profile: string | null): string {
  const p = (profile ?? "").trim();
  if (!p) {
    return "Voice profile: unavailable — write in a plain, direct imperative style.";
  }
  return `Voice profile (mimic this operator's style):\n${p.slice(0, 800)}`;
}

export interface GenSignals {
  lastAssistantTail?: string;
  verifyOk?: boolean;
  verifyKnown?: boolean;
  dirtyFiles?: number;
  taskTail?: string;
}

export function buildGenPrompt(signals: GenSignals, voice: string | null): string {
  const parts = [
    "You predict a coder's NEXT PROMPT after a finished agent turn.",
    voiceBlock(voice),
    "Rules: one single line, ≤ 240 chars, the exact wording this operator would type.",
    "No quotes, no code fences, no explanation. If nothing sensible follows, output SKIP.",
    "",
    "Signals:",
  ];
  if (signals.verifyKnown) {
    parts.push(`- verify gates: ${signals.verifyOk ? "green" : "FAILING"}`);
  }
  if (signals.dirtyFiles !== undefined) {
    parts.push(`- worktree: ${signals.dirtyFiles} changed file(s)`);
  }
  if (signals.taskTail) parts.push(`- task list tail: ${signals.taskTail.slice(0, 200)}`);
  if (signals.lastAssistantTail) {
    parts.push(`- assistant's last reply ended with:\n  ${signals.lastAssistantTail.slice(0, 400)}`);
  }
  parts.push("", "Next prompt (or SKIP):");
  return parts.join("\n");
}

/** Atomic write of the staged suggestion for the TUI ghost text. */
export function writeSuggestion(dir: string, doc: SuggestionDoc): string {
  mkdirSync(dir, { recursive: true });
  const path = join(dir, "prompt-suggestion.json");
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(doc)}\n`);
  renameSync(tmp, path);
  return path;
}

export function clearSuggestion(dir: string): void {
  try {
    writeSuggestion(dir, {
      version: 1,
      id: "cleared",
      text: "",
      source: "heuristic",
      created: new Date().toISOString(),
      expires: 0,
    });
  } catch {
    // cosmetic
  }
}
