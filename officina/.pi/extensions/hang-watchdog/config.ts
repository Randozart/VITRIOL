// Decision-lane configuration (2026-11-16).
//
// The typed SystemOne judge (`decision.ts`) reads its endpoint/key/floor from
// env vars. This module gives the operator a first-class way to set them
// without hand-editing launcher files: a 0600 JSON config under the officina
// directory that judgeWithDecision consults (env vars still win as an
// override for throwaway runs).
//
// PROVENANCE: original Officina work (this repo).

import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/** Which judge answers before the heuristics give up (2026-11-16). */
export type DecisionMode = "heuristic" | "http" | "gemini";

export interface DecisionConfig {
  version: 1;
  /**
   * `heuristic` — local only (default).
   * `http`      — POST a SystemOne-shaped endpoint (url).
   * `gemini`    — Gemini Decision 1, direct (calibrated probabilities).
   */
  mode?: DecisionMode;
  /** SystemOne endpoint (mode=http), e.g. http://127.0.0.1:8000/v1/systemone. */
  url: string;
  /** Bearer token for mode=http, or the Gemini API key for mode=gemini. */
  key: string;
  /** answer-confidence / act-probability floor (0..1). */
  floor: number;
  /** Required lead over the runner-up (mode=gemini), 0..1. */
  margin?: number;
  /** Decision model id (mode=gemini). */
  model?: string;
  /** Scrub evidence before it leaves the box (mode=gemini). Default on. */
  redact?: boolean;
}

/** `~/.vitriol/officina/watchdog-decision.json` — never logs the key. */
export function decisionConfigPath(
  dir = join(homedir(), ".vitriol", "officina"),
): string {
  // A full-path override keeps tests and throwaway runs out of the operator's
  // real config (2026-11-16).
  const override = (process.env.OFFICINA_WATCHDOG_DECISION_FILE ?? "").trim();
  if (override) return override;
  return join(dir, "watchdog-decision.json");
}

/** Read the config; missing/corrupt → null (the env fallback still works). */
export function readDecisionConfig(dir?: string): DecisionConfig | null {
  try {
    const doc = JSON.parse(
      readFileSync(decisionConfigPath(dir), "utf8"),
    ) as Partial<DecisionConfig>;
    if (doc.version !== 1) return null;
    const mode =
      doc.mode === "gemini" || doc.mode === "http" || doc.mode === "heuristic"
        ? doc.mode
        : undefined;
    // An empty URL is tolerated (the lane then resolves to `heuristic`); a
    // gemini-only config legitimately has no endpoint at all.
    const url = typeof doc.url === "string" ? doc.url.trim() : "";
    const floor = Number(doc.floor);
    const margin = Number(doc.margin);
    return {
      version: 1,
      ...(mode ? { mode } : {}),
      url,
      key: typeof doc.key === "string" ? doc.key : "",
      floor: Number.isFinite(floor) && floor >= 0 && floor <= 1 ? floor : 0.9,
      ...(Number.isFinite(margin) && margin >= 0 && margin <= 1 ? { margin } : {}),
      ...(typeof doc.model === "string" && doc.model.trim()
        ? { model: doc.model.trim() }
        : {}),
      ...(typeof doc.redact === "boolean" ? { redact: doc.redact } : {}),
    };
  } catch {
    return null;
  }
}

/** Atomically write the config; failures are silent (config is convenience). */
export function writeDecisionConfig(
  cfg: DecisionConfig,
  dir?: string,
): boolean {
  try {
    const path = decisionConfigPath(dir);
    mkdirSync(join(path, ".."), { recursive: true });
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, JSON.stringify(cfg, null, 2) + "\n", { mode: 0o600 });
    renameSync(tmp, path);
    return true;
  } catch {
    return false;
  }
}

/**
 * A Gemini key from the ascensus secret store (`~/.vitriol/secrets`). Mirrors
 * ascensusd's own sniffing: a bare `api_key` beginning `AIza` is a Gemini key,
 * and a `[gemini]` section (future multi-provider format) is preferred.
 */
export function ascensusGeminiKey(
  path = join(homedir(), ".vitriol", "secrets"),
  read: (p: string) => string = (p) => readFileSync(p, "utf8"),
): string {
  let text = "";
  try {
    text = read(path);
  } catch {
    return "";
  }
  let section = "";
  let bare = "";
  let inGemini = "";
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const header = /^\[([^\]]+)\]$/.exec(line);
    if (header) {
      section = header[1].trim().toLowerCase();
      continue;
    }
    const m = /^(\w+)\s*=\s*(.+)$/.exec(line);
    if (!m) continue;
    const key = m[1].toLowerCase();
    const value = m[2].trim().replace(/^["']|["']$/g, "");
    if (!value) continue;
    if (section === "gemini" && (key === "api_key" || key === "key")) inGemini = value;
    if (section === "" && key === "api_key" && value.startsWith("AIza")) bare = value;
    if (section === "" && (key === "gemini_api_key" || key === "gemini_key")) bare = value;
  }
  return inGemini || bare;
}

/** Everything the lane needs: env override first, then the file. */
export function resolveDecisionConfig(env: NodeJS.ProcessEnv = process.env): {
  mode: DecisionMode;
  endpoint: string;
  key: string;
  floor: number;
  margin: number;
  model: string;
  redact: boolean;
} {
  const file = readDecisionConfig();
  const endpoint =
    (env.OFFICINA_WATCHDOG_DECISION_URL ?? "").trim() || file?.url || "";
  const explicitMode = String(env.OFFICINA_WATCHDOG_DECISION_MODE ?? "")
    .trim()
    .toLowerCase();
  const mode: DecisionMode =
    explicitMode === "gemini" || explicitMode === "http"
      ? explicitMode
      : file?.mode ?? (endpoint ? "http" : "heuristic");
  const configured =
    (env.OFFICINA_WATCHDOG_DECISION_KEY ?? "").trim() || file?.key || "";
  // For the Gemini lane the key may also live in the environment (the usual
  // place) or in ascensus's secret store, so one key serves both systems.
  const key =
    mode === "gemini"
      ? configured ||
        String(env.GEMINI_API_KEY ?? "").trim() ||
        ascensusGeminiKey()
      : configured;
  const minScore = Number(env.OFFICINA_WATCHDOG_DECISION_MIN_SCORE);
  const marginEnv = Number(env.OFFICINA_WATCHDOG_DECISION_MARGIN);
  const redactEnv = String(env.OFFICINA_WATCHDOG_DECISION_REDACT ?? "").trim();
  return {
    mode,
    endpoint,
    key,
    floor: Number.isFinite(minScore) ? minScore : (file?.floor ?? 0.9),
    margin: Number.isFinite(marginEnv) ? marginEnv : (file?.margin ?? 0.15),
    model:
      (env.OFFICINA_WATCHDOG_DECISION_MODEL ?? "").trim() ||
      file?.model ||
      "gemini-decision-1",
    redact: redactEnv === "0" || /^false$/i.test(redactEnv)
      ? false
      : (file?.redact ?? true),
  };
}
