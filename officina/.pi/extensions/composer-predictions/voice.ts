// Voice learning rules (voice-suggestions stage 3, 2026-11-12). Pure +
// injectable. I4: only auto-route-SAFE prompts ever reach a cloud lane for
// distillation; acceptance telemetry contributes ids/verdicts only.

import { readFileSync } from "node:fs";

export interface VoiceProfile {
  version: 1;
  updatedAt: string;
  profile: string;
  samples: number;
}

export const DISTILL_EVERY_PROMPTS = 40;
export const IDLE_MS_DEFAULT = 5 * 60_000;
export const MAX_PROFILE_WORDS = 200;
const MAX_SAFE_PROMPTS = 30;
const MAX_ACCEPTED = 20;

/** Distill when enough new safe prompts accumulated, or once a day. */
export function shouldDistill(
  profile: VoiceProfile | null,
  safePrompts: number,
  now = new Date(),
): { ok: boolean; reason: string } {
  if (safePrompts < 5) {
    return { ok: false, reason: "not enough safe prompts yet" };
  }
  if (!profile) return { ok: true, reason: "no profile yet" };
  const day = now.toISOString().slice(0, 10);
  const profileDay = (profile.updatedAt || "").slice(0, 10);
  const sinceSamples = safePrompts - (profile.samples ?? 0);
  if (profileDay === day && sinceSamples < DISTILL_EVERY_PROMPTS) {
    return { ok: false, reason: `profile fresh today (${sinceSamples} new prompts)` };
  }
  return { ok: true, reason: `${sinceSamples} new safe prompts since last distill` };
}

export function readProfile(path: string): VoiceProfile | null {
  try {
    const doc = JSON.parse(readFileSync(path, "utf8")) as VoiceProfile;
    if (doc.version !== 1 || typeof doc.profile !== "string") return null;
    return doc;
  } catch {
    return null;
  }
}

const SECRET_PATTERNS: Array<[RegExp, string]> = [
  [/\bsk-[A-Za-z0-9_-]{8,}/g, "sk-REDACTED"],
  [/\bgh[pousr]_[A-Za-z0-9]{16,}/g, "gh_TOKEN"],
  [/\bgithub_pat_[A-Za-z0-9_]{20,}/g, "gh_pat_REDACTED"],
  [/\bAKIA[0-9A-Z]{16}/g, "AKIA_REDACTED"],
  [/\bxox[baprs]-[A-Za-z0-9-]{10,}/g, "xox-REDACTED"],
  [/\bBearer\s+[A-Za-z0-9._~+/=-]{8,}/gi, "Bearer REDACTED"],
  [/\beyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}/g, "JWT_REDACTED"],
  [/\b[0-9a-fA-F]{32,}\b/g, "HEX_REDACTED"],
];

/** Defense-in-depth (Codex-parity): scrub key-shaped strings from
 *  GENERATED text before it is persisted or shown (I4). */
export function redactSecrets(text: string): string {
  let out = String(text ?? "");
  for (const [rx, replacement] of SECRET_PATTERNS) {
    out = out.replace(rx, replacement);
  }
  return out;
}

/** Acceptance events → compact stats block (ids and verdicts only, I4). */
export function acceptanceBlock(events: Array<{ verdict?: string; source?: string }>): string {
  const total = events.length;
  const accepted = events.filter((e) => e.verdict === "accepted").length;
  const bySource: Record<string, number> = {};
  for (const e of events) {
    if (e.verdict === "accepted") {
      const s = String(e.source ?? "?");
      bySource[s] = (bySource[s] ?? 0) + 1;
    }
  }
  const src = Object.entries(bySource)
    .map(([k, v]) => `${k}:${v}`)
    .join(", ");
  return `Acceptance telemetry: ${accepted}/${total} suggestions accepted${src ? ` (${src})` : ""}.`;
}

export function buildDistillPrompt(
  profile: VoiceProfile | null,
  safePrompts: string[],
  accepted: string[],
): string {
  const prompts = safePrompts.slice(-MAX_SAFE_PROMPTS).map((p) => `- ${p.slice(0, 200)}`);
  const acc = accepted.slice(-MAX_ACCEPTED).map((p) => `- ${p.slice(0, 200)}`);
  return [
    "You maintain a VOICE PROFILE for a coder: how they phrase prompts to their coding agent.",
    "Fold the evidence below into an updated profile. Keep what still holds, drop what new evidence contradicts.",
    profile ? `Current profile:\n${profile.profile}` : "Current profile: (none yet — create it from the evidence)",
    "",
    `The operator's own recent prompts (privacy-screened):\n${prompts.join("\n") || "(none)"}`,
    "",
    `Suggestions they ACCEPTED into their composer (learn taste from these too):\n${acc.join("\n") || "(none)"}`,
    "",
    `Output the updated profile as plain prose, at most ${MAX_PROFILE_WORDS} words.`,
    "Cover: tone (terse? polite?), register (imperative? first person?), typical verbs, sentence length,",
    "language(s), formatting habits, recurring topics. No preamble, no fences.",
  ].join("\n");
}

/** Plain-prose profile extraction, clamped to MAX_PROFILE_WORDS. */
export function parseProfile(reply: string | null): string | null {
  if (!reply) return null;
  let text = String(reply).trim();
  const fence = /```(?:\w+)?\n([\s\S]*?)```/.exec(text);
  if (fence) text = fence[1].trim();
  text = text.replace(/^profile\s*:\s*/i, "");
  const words = text.split(/\s+/).filter(Boolean);
  if (words.length === 0) return null;
  return words.slice(0, MAX_PROFILE_WORDS).join(" ");
}
