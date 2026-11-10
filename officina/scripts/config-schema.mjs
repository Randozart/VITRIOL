// Harness config schema + provenance fingerprint (verification-rollout T0.3).
//
// WHY THIS EXISTS
// The 2026-09-03 MTP post-mortem: a config retarget silently dropped the
// `[spec]` section and the engine lost 40% decode for weeks. Nothing
// ASSERTED the speed-bearing keys were still present — the fingerprint
// recorded what it covered, and stayed silent about what it didn't.
//
// This module is the harness-side equivalent for `~/.vitriol/officina/env`
// and `~/.pi/agent/settings.json`:
//   - a schema declaring which keys must exist (silently absent = changed
//     behaviour, not a default),
//   - a stable fingerprint over the provenance-bearing subset,
//   - a diff so a drift is REPORTED rather than discovered weeks later.
//
// PURE: no filesystem access. `config-drift.mjs` does the I/O.

import { createHash } from "node:crypto";
import { parseEnvFile } from "./caduceus-lib.mjs";

/** Severity for a missing key. `required` fails the gate; `expected` warns. */
export const REQUIRED = "required";
export const EXPECTED = "expected";

/**
 * Env keys whose SILENT ABSENCE changes behaviour (routing policy, safety
 * mode, gateway wiring, context-trimming). `why` is the failure mode if it
 * disappears — the post-mortem line, in one sentence.
 */
export const ENV_SCHEMA = [
  {
    key: "OFFICINA_PERMISSIONS_MODE",
    level: REQUIRED,
    why: "tool gating falls back to defaults; yolo/ask policy silently changes",
    provenance: true,
  },
  {
    key: "OFFICINA_CADUCEUS",
    level: REQUIRED,
    why: "gateway integration turns off; cloud lanes vanish and routing degrades to local-only",
    provenance: true,
  },
  {
    key: "OFFICINA_OVERFLOW",
    level: REQUIRED,
    why: "busy-engine spillover stops; turns queue behind the local engine instead",
    provenance: true,
  },
  {
    key: "OFFICINA_OVERFLOW_CLOUD",
    level: REQUIRED,
    why: "overflow has no candidate source and silently does nothing",
    provenance: true,
  },
  {
    key: "OFFICINA_OVERFLOW_FAILOVER",
    level: REQUIRED,
    why: "engine-down failover stops; every turn fails while the engine is down",
    provenance: true,
  },
  {
    key: "OFFICINA_OVERFLOW_QUEUE_MS",
    level: EXPECTED,
    why: "overflow fires on the default queue wait, spilling turns that would have been served locally",
    provenance: true,
  },
  {
    key: "OFFICINA_ROUTE_MODE",
    level: REQUIRED,
    why: "auto-route reverts to its default advisory mode",
    provenance: true,
  },
  {
    key: "OFFICINA_ROUTE_THRESHOLD",
    level: EXPECTED,
    why: "routing threshold reverts to 0.5 (more cloud than intended)",
    provenance: true,
  },
  {
    key: "OFFICINA_CACHE_MODE",
    level: EXPECTED,
    why: "prompt-cache policy reverts to auto",
    provenance: true,
  },
  {
    key: "LITTLE_CODER_CLEAR_KEEP",
    level: EXPECTED,
    why: "stale tool results stay in context and burn the window (perf 2026-11-10)",
    provenance: true,
  },
  {
    key: "OFFICINA_PRAETOR",
    level: EXPECTED,
    why: "verification gate silently re-enables/disables in the editor",
    provenance: false,
  },
];

/**
 * settings.json paths that must exist. NOTE: pi keys modelOverrides by the
 * FLAT combined string "provider/model" (e.g. "llamacpp/Lapis Occultus"), not
 * as nested objects — verified against the live file 2026-11-10.
 * Compaction reserves are load-bearing:
 * without them pi compacts at its default reserve and a 197k-context session
 * overflows before compaction fires.
 */
export const SETTINGS_SCHEMA = [
  {
    path: ["defaultProvider"],
    level: REQUIRED,
    why: "new sessions lose the Lapis Occultus default and open on whatever provider pi picks",
    provenance: true,
  },
  {
    path: ["defaultModel"],
    level: REQUIRED,
    why: "new sessions lose the resilient virtual default",
    provenance: true,
  },
  {
    path: ["compaction", "modelOverrides", "llamacpp/Lapis Occultus", "reserveTokens"],
    level: EXPECTED,
    why: "compaction fires too late at depth and the turn overflows the window",
    provenance: true,
  },
  {
    path: ["compaction", "modelOverrides", "llamacpp/Lapis Occultus", "keepRecentTokens"],
    level: EXPECTED,
    why: "compaction keeps too much history and re-fills the window it just freed",
    provenance: true,
  },
  {
    path: ["compaction", "modelOverrides", "officina/lapis", "reserveTokens"],
    level: EXPECTED,
    why: "the virtual route compacts at pi defaults (same overflow risk as above)",
    provenance: true,
  },
  {
    path: ["compaction", "modelOverrides", "officina/lapis", "keepRecentTokens"],
    level: EXPECTED,
    why: "the virtual route keeps too much history after compaction",
    provenance: true,
  },
];

/**
 * Resolve a dotted path inside a parsed settings object.
 *
 * @param {unknown} obj
 * @param {string[]} path
 * @returns {unknown} the value, or undefined when any segment is missing
 */
export function getPath(obj, path) {
  let cur = obj;
  for (const seg of path) {
    if (cur === null || typeof cur !== "object" || Array.isArray(cur)) return undefined;
    cur = cur[seg];
  }
  return cur;
}

/**
 * Validate a parsed env map against ENV_SCHEMA.
 *
 * @param {Map<string,string>|Record<string,string>} env
 * @returns {{errors: Array<{key:string,why:string}>, warnings: Array<{key:string,why:string}>, ok: boolean}}
 */
export function validateEnv(env) {
  const values = env instanceof Map ? env : new Map(Object.entries(env ?? {}));
  const errors = [];
  const warnings = [];
  for (const entry of ENV_SCHEMA) {
    // An empty value is as silent as a missing key.
    const raw = values.get(entry.key);
    if (raw === undefined || String(raw).trim() === "") {
      (entry.level === REQUIRED ? errors : warnings).push({
        key: entry.key,
        why: entry.why,
      });
    }
  }
  return { errors, warnings, ok: errors.length === 0 };
}

/**
 * Validate parsed settings JSON against SETTINGS_SCHEMA.
 *
 * @param {unknown} settings
 * @returns {{errors: Array<{key:string,why:string}>, warnings: Array<{key:string,why:string}>, ok: boolean}}
 */
export function validateSettings(settings) {
  const errors = [];
  const warnings = [];
  for (const entry of SETTINGS_SCHEMA) {
    const value = getPath(settings, entry.path);
    if (value === undefined || value === null || value === "") {
      (entry.level === REQUIRED ? errors : warnings).push({
        key: entry.path.join("."),
        why: entry.why,
      });
    }
  }
  return { errors, warnings, ok: errors.length === 0 };
}

/**
 * Merge env + settings findings into one report.
 *
 * @param {{env?: Map<string,string>|null, settings?: unknown}} input
 */
export function validateConfig({ env = null, settings = undefined } = {}) {
  const envReport = env ? validateEnv(env) : { errors: [], warnings: [], ok: true };
  const settingsReport = validateSettings(settings);
  const errors = [...envReport.errors, ...settingsReport.errors];
  const warnings = [...envReport.warnings, ...settingsReport.warnings];
  return { errors, warnings, ok: errors.length === 0 };
}

/**
 * Provenance-bearing env pairs, sorted, restricted to the schema.
 * Secrets never enter a fingerprint: only keys declared in the schema.
 *
 * @param {Map<string,string>|Record<string,string>} env
 * @returns {Array<[string,string]>}
 */
export function provenanceEnvPairs(env) {
  const values = env instanceof Map ? env : new Map(Object.entries(env ?? {}));
  const pairs = [];
  for (const entry of ENV_SCHEMA) {
    if (!entry.provenance) continue;
    const raw = values.get(entry.key);
    if (raw === undefined || String(raw).trim() === "") continue;
    pairs.push([entry.key, String(raw).trim()]);
  }
  return pairs.sort((a, b) => a[0].localeCompare(b[0]));
}

/**
 * Provenance-bearing settings pairs, sorted, restricted to the schema.
 *
 * @param {unknown} settings
 * @returns {Array<[string,string]>}
 */
export function provenanceSettingsPairs(settings) {
  const pairs = [];
  for (const entry of SETTINGS_SCHEMA) {
    if (!entry.provenance) continue;
    const value = getPath(settings, entry.path);
    if (value === undefined || value === null || value === "") continue;
    pairs.push([entry.path.join("."), String(value)]);
  }
  return pairs.sort((a, b) => a[0].localeCompare(b[0]));
}

/**
 * Stable fingerprint of the provenance-bearing config (sorted keys, sha256).
 *
 * @param {{env?: Map<string,string>|Record<string,string>, settings?: unknown}} input
 * @returns {{hash: string, pairs: Array<[string,string]>}}
 */
export function fingerprintConfig({ env = {}, settings = undefined } = {}) {
  const pairs = [...provenanceEnvPairs(env), ...provenanceSettingsPairs(settings)];
  pairs.sort((a, b) => a[0].localeCompare(b[0]));
  const canonical = pairs.map(([k, v]) => `${k}=${v}`).join("\n");
  return {
    hash: createHash("sha256").update(canonical).digest("hex").slice(0, 16),
    pairs,
  };
}

/**
 * Diff two fingerprint pair lists ("last known good" -> live).
 *
 * @param {Array<[string,string]>} previous
 * @param {Array<[string,string]>} current
 * @returns {{added: Array<[string,string]>, removed: Array<[string,string]>, changed: Array<{key:string,from:string,to:string}>, clean: boolean}}
 */
export function diffFingerprints(previous, current) {
  const prev = new Map(previous ?? []);
  const cur = new Map(current ?? []);
  const added = [];
  const removed = [];
  const changed = [];
  for (const [key, value] of cur) {
    if (!prev.has(key)) added.push([key, value]);
    else if (prev.get(key) !== value) changed.push({ key, from: prev.get(key), to: value });
  }
  for (const [key, value] of prev) {
    if (!cur.has(key)) removed.push([key, value]);
  }
  added.sort((a, b) => a[0].localeCompare(b[0]));
  removed.sort((a, b) => a[0].localeCompare(b[0]));
  changed.sort((a, b) => a.key.localeCompare(b.key));
  return { added, removed, changed, clean: added.length + removed.length + changed.length === 0 };
}

export { parseEnvFile };
