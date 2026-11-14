// Gateway cooldown facts (2026-11-14) — cross-session knowledge of lanes the
// CADUCEUS GATEWAY refuses.
//
// The gateway cools credentials on its own schedule and reports it only in the
// 429 body (`reset_seconds` / `retry_after` / `reset_at`), not in a header and
// not in our provider-health file. Without this, every session re-discovers the
// same refusal: pick lane -> 429 -> cool 30 min locally -> pick it again.
//
// Extensions publish marks here (atomic facts file, mode 600); every router
// reads them and skips marked lanes until `until`. Cap + staleness keep a mark
// from outliving its truth (lane-spread re-discovers after the cap).

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { cooldownCeilingMs } from "./provider-health.ts";

export interface GatewayMark {
  until: number;
  reason: string;
}

export interface GatewayMarksFile {
  version: 1;
  ts: number;
  /** Model id within a provider, e.g. "freeinference/deepseek-v4-flash". */
  models: Record<string, GatewayMark>;
  /** Owner id, e.g. "freeinference" (covers every model of that owner). */
  owners: Record<string, GatewayMark>;
  /** Model ids the upstream says it does not offer (404 model_not_found).
   *  Longer-lived than a 429: the catalog simply has a stale name. */
  invalid?: Record<string, GatewayMark>;
  /** Learned real context limits (provider 400 "maximum context length is
   *  N tokens"). The catalog often overstates the window. */
  limits?: Record<string, { tokens: number; learnedAt: number }>;
}

export function gatewayHealthPath(env: NodeJS.ProcessEnv = process.env): string {
  if (env.OFFICINA_GATEWAY_HEALTH_FILE) return env.OFFICINA_GATEWAY_HEALTH_FILE;
  return join(env.HOME || homedir(), ".vitriol", "officina", "gateway-health.json");
}

function emptyFile(now: number): GatewayMarksFile {
  return { version: 1, ts: now, models: {}, owners: {} };
}

function readFileAt(path: string): GatewayMarksFile {
  try {
    const parsed = JSON.parse(readFileSync(path, "utf8")) as Partial<GatewayMarksFile>;
    if (!parsed || parsed.version !== 1) return emptyFile(Date.now());
    return {
      version: 1,
      ts: Number(parsed.ts) || 0,
      models: (parsed.models ?? {}) as Record<string, GatewayMark>,
      owners: (parsed.owners ?? {}) as Record<string, GatewayMark>,
      invalid: (parsed.invalid ?? {}) as Record<string, GatewayMark>,
      limits: (parsed.limits ?? {}) as Record<string, { tokens: number; learnedAt: number }>,
    };
  } catch {
    return emptyFile(Date.now());
  }
}

function prune(file: GatewayMarksFile, now: number): GatewayMarksFile {
  const keep = (marks: Record<string, GatewayMark>): Record<string, GatewayMark> => {
    const out: Record<string, GatewayMark> = {};
    for (const [key, mark] of Object.entries(marks ?? {})) {
      if (mark && Number(mark.until) > now) out[key] = mark;
    }
    return out;
  };
  file.models = keep(file.models);
  file.owners = keep(file.owners);
  file.invalid = keep(file.invalid ?? {});
  return file;
}

export interface GatewayMarks {
  models: Set<string>;
  owners: Set<string>;
  /** Lanes the upstream rejected as not-offered (stale catalog ids). */
  invalid: Set<string>;
  /** Learned effective context windows (tokens). */
  limits: Record<string, number>;
}

/** Lanes/owners currently refused by the gateway (expired marks pruned). */
export function readGatewayMarks(
  now: number = Date.now(),
  env: NodeJS.ProcessEnv = process.env,
): GatewayMarks {
  const file = prune(readFileAt(gatewayHealthPath(env)), now);
  return {
    models: new Set(Object.keys(file.models)),
    owners: new Set(Object.keys(file.owners)),
    invalid: new Set(Object.keys(file.invalid ?? {})),
    limits: Object.fromEntries(
      Object.entries(file.limits ?? {}).map(([k, v]) => [k, Number(v?.tokens) || 0]),
    ),
  };
}

export interface MarkInput {
  /** Model id inside the provider, e.g. "freeinference/deepseek-v4-flash". */
  model?: string;
  /** Owner-level mark (cool every model of the owner). */
  owner?: string;
  untilMs: number;
  reason: string;
}

/** Record a gateway refusal (atomic merge; clamps to the cooldown ceiling). */
export function markGatewayCooldown(input: MarkInput, env: NodeJS.ProcessEnv = process.env): number {
  const now = Date.now();
  const cap = cooldownCeilingMs(env);
  const until = Math.min(now + cap, Math.max(now + 60_000, Math.floor(input.untilMs)));
  const path = gatewayHealthPath(env);
  const file = prune(readFileAt(path), now);
  if (input.model) file.models[input.model] = { until, reason: input.reason };
  if (input.owner) file.owners[input.owner] = { until, reason: input.reason };
  file.ts = now;
  try {
    mkdirSync(join(path, ".."), { recursive: true, mode: 0o700 });
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(file, null, 2)}\n`, { mode: 0o600 });
    renameSync(tmp, path);
  } catch {
    // Decoration contract: a failed mark never breaks a request.
  }
  return until;
}

/**
 * Cooldown implied by a provider error message. The gateway sends
 * `{"reset_seconds":21883,...}` / `"retry_after":"<iso>"` / `"reset_at"` in
 * the BODY; our headers-based recordRetryAfter misses it. Returns ms clamped
 * to the cooldown ceiling, or undefined.
 */
export function parseCooldownFromError(
  text: string | undefined,
  now: number = Date.now(),
  env: NodeJS.ProcessEnv = process.env,
): number | undefined {
  if (!text) return undefined;
  const cap = cooldownCeilingMs(env);
  const clamp = (ms: number): number => Math.min(cap, Math.max(60_000, Math.floor(ms)));
  const seconds = /"reset_seconds"\s*:\s*(\d+)/.exec(text);
  if (seconds) return clamp(Number(seconds[1]) * 1000);
  const iso = /"(?:retry_after|reset_at)"\s*:\s*"([^"]+)"/.exec(text);
  if (iso) {
    const at = Date.parse(iso[1]!);
    if (Number.isFinite(at)) return clamp(at - now);
  }
  return undefined;
}

/** How long a stale catalog id stays excluded (catalog refresh should fix
 *  it sooner; this is the safety net). */
export function invalidCeilingMs(env: NodeJS.ProcessEnv = process.env): number {
  const n = Number(env.OFFICINA_GATEWAY_INVALID_MAX_MS);
  return Number.isFinite(n) && n > 0 ? Math.max(5 * 60_000, Math.floor(n)) : 6 * 3600_000;
}

/** Mark a lane as not-offered by the upstream (404 model_not_found). */
export function markGatewayInvalid(
  input: { model: string; reason: string; untilMs?: number },
  env: NodeJS.ProcessEnv = process.env,
): number {
  const now = Date.now();
  const cap = invalidCeilingMs(env);
  const until = Math.min(now + cap, Math.max(now + 5 * 60_000, Math.floor(input.untilMs ?? now + cap)));
  const path = gatewayHealthPath(env);
  const file = prune(readFileAt(path), now);
  file.invalid = file.invalid ?? {};
  file.invalid[input.model] = { until, reason: input.reason.slice(0, 300) };
  file.ts = now;
  try {
    mkdirSync(join(path, ".."), { recursive: true, mode: 0o700 });
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(file, null, 2)}\n`, { mode: 0o600 });
    renameSync(tmp, path);
  } catch {
    // decoration contract
  }
  return until;
}

export interface ModelNotFoundFact {
  missing: string;
  alternatives: string[];
}

/** Parse the gateway/upstream 404 (model_not_found) message. */
export function parseModelNotFound(text: string | undefined): ModelNotFoundFact | undefined {
  if (!text) return undefined;
  if (!/model_not_found|not currently offered/i.test(text)) return undefined;
  const clean = (s: string): string => s.trim().replace(/[.,;:]+$/, "");
  const missing = clean(/not currently offered:?\s*([^\s",]+)/i.exec(text)?.[1] ?? "");
  const list = /Available models:\s*([^"]+)/i.exec(text)?.[1] ?? "";
  const alternatives = list
    .split(",")
    .map(clean)
    .filter((s) => s.length > 0 && s.length < 120);
  return { missing, alternatives };
}

/** Parse a provider context-length 400 ("maximum context length is 180000
 *  tokens"). Returns the effective window (output reserve already included
 *  by the provider's own message). */
export function parseContextLimit(text: string | undefined): number | undefined {
  if (!text) return undefined;
  // Variant 1 (upstream): "maximum context length is 180000 tokens".
  // Variant 2 (gateway): "estimated 183106 input tokens, limit 128000".
  const m =
    /maximum context length is\s+(\d{3,})\s+tokens/i.exec(text) ??
    /\blimit\s+(\d{4,})\b/i.exec(text);
  if (!m) return undefined;
  const tokens = Number(m[1]);
  return Number.isFinite(tokens) && tokens >= 1024 ? Math.floor(tokens) : undefined;
}

/** Prompt size quoted by a context-overflow 400 ("value=171809",
 *  "estimated 183106 input tokens"). */
export function parseOverflowPromptTokens(text: string | undefined): number | undefined {
  if (!text) return undefined;
  const m =
    /value=(\d{3,})/i.exec(text) ??
    /estimated\s+(\d{3,})\s+input tokens/i.exec(text) ??
    /prompt contains at least\s+(\d{3,})/i.exec(text);
  if (!m) return undefined;
  const tokens = Number(m[1]);
  return Number.isFinite(tokens) && tokens >= 1024 ? Math.floor(tokens) : undefined;
}

/** Learn a model's real window (idempotent; keeps the smaller value). */
export function markModelLimit(
  modelId: string,
  tokens: number,
  env: NodeJS.ProcessEnv = process.env,
): void {
  const id = String(modelId ?? "").trim();
  const value = Math.floor(Number(tokens));
  if (!id || !Number.isFinite(value) || value < 1024) return;
  const now = Date.now();
  const path = gatewayHealthPath(env);
  const file = prune(readFileAt(path), now);
  file.limits = file.limits ?? {};
  const prev = file.limits[id]?.tokens ?? Number.POSITIVE_INFINITY;
  file.limits[id] = { tokens: Math.min(prev, value), learnedAt: now };
  file.ts = now;
  try {
    mkdirSync(join(path, ".."), { recursive: true, mode: 0o700 });
    const tmp = `${path}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(file, null, 2)}\n`, { mode: 0o600 });
    renameSync(tmp, path);
  } catch {
    // decoration contract
  }
}

/** Learned limit for a lane (undefined = use the catalog window). */
export function modelLimit(
  modelId: string,
  env: NodeJS.ProcessEnv = process.env,
): number | undefined {
  const limits = readGatewayMarks(Date.now(), env).limits;
  const value = limits[modelId];
  return Number.isFinite(value) && value! > 0 ? value : undefined;
}

/** Owner part of a lane id ("freeinference/deepseek-v4-flash" → "freeinference"). */
export function ownerOfLane(modelId: string): string {
  return String(modelId ?? "").split("/")[0]!.trim().toLowerCase();
}

/** Test helper: wipe the shared facts file. */
export function resetGatewayMarks(env: NodeJS.ProcessEnv = process.env): void {
  const path = gatewayHealthPath(env);
  try {
    if (existsSync(path)) writeFileSync(path, `${JSON.stringify(emptyFile(Date.now()))}\n`);
  } catch {
    // ignore
  }
}
