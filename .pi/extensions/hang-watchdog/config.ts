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

export interface DecisionConfig {
  version: 1;
  /** SystemOne endpoint, e.g. http://127.0.0.1:8000/v1/systemone. */
  url: string;
  /** Optional Bearer token for the endpoint. */
  key: string;
  /** answer-confidence / act-probability floor (0..1). */
  floor: number;
}

/** `~/.vitriol/officina/watchdog-decision.json` — never logs the key. */
export function decisionConfigPath(
  dir = join(homedir(), ".vitriol", "officina"),
): string {
  return join(dir, "watchdog-decision.json");
}

/** Read the config; missing/corrupt → null (the env fallback still works). */
export function readDecisionConfig(dir?: string): DecisionConfig | null {
  try {
    const doc = JSON.parse(
      readFileSync(decisionConfigPath(dir), "utf8"),
    ) as Partial<DecisionConfig>;
    if (doc.version !== 1 || typeof doc.url !== "string") return null;
    const floor = Number(doc.floor);
    return {
      version: 1,
      url: doc.url.trim(),
      key: typeof doc.key === "string" ? doc.key : "",
      floor: Number.isFinite(floor) && floor >= 0 && floor <= 1 ? floor : 0.9,
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

/** Endpoint+floor+key for this process: env override first, then the file. */
export function resolveDecisionConfig(env: NodeJS.ProcessEnv = process.env): {
  endpoint: string;
  key: string;
  floor: number;
} {
  const file = readDecisionConfig();
  return {
    endpoint:
      (env.OFFICINA_WATCHDOG_DECISION_URL ?? "").trim() || file?.url || "",
    key: (env.OFFICINA_WATCHDOG_DECISION_KEY ?? "").trim() || file?.key || "",
    floor: Number.isFinite(Number(env.OFFICINA_WATCHDOG_DECISION_MIN_SCORE))
      ? Number(env.OFFICINA_WATCHDOG_DECISION_MIN_SCORE)
      : (file?.floor ?? 0.9),
  };
}
