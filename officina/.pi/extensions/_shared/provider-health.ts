// Persistent provider/model health state. Survives TUI restarts so a
// rate-limited or dead lane is not retried every session.

import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export interface HealthRecord {
  provider: string;
  model: string;
  status: "ok" | "cooling" | "unknown";
  cooldownUntil: number;
  lastSuccessAt?: number;
  lastFailureAt?: number;
  lastStatus?: number;
  errorClass?: string;
  successCount: number;
  failureCount: number;
}

interface HealthFile {
  version: 1;
  models: Record<string, HealthRecord>;
  owners: Record<string, { cooldownUntil: number; errorClass?: string; lastStatus?: number }>;
}

const DEFAULT: HealthFile = { version: 1, models: {}, owners: {} };

function path(): string {
  if (process.env.OFFICINA_PROVIDER_HEALTH_FILE) return process.env.OFFICINA_PROVIDER_HEALTH_FILE;
  return join(homedir(), ".vitriol", "officina", "provider-health.json");
}

function load(): HealthFile {
  try {
    if (!existsSync(path())) return { ...DEFAULT, models: {}, owners: {} };
    const parsed = JSON.parse(readFileSync(path(), "utf8"));
    return {
      version: 1,
      models: parsed?.models && typeof parsed.models === "object" ? parsed.models : {},
      owners: parsed?.owners && typeof parsed.owners === "object" ? parsed.owners : {},
    };
  } catch {
    return { ...DEFAULT, models: {}, owners: {} };
  }
}

function save(file: HealthFile): void {
  try {
    mkdirSync(dirname(path()), { recursive: true, mode: 0o700 });
    const tmp = `${path()}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(file, null, 2)}\n`, { mode: 0o600 });
    chmodSync(tmp, 0o600);
    renameSync(tmp, path());
    chmodSync(path(), 0o600);
  } catch {
    // health persistence is best-effort
  }
}

function prune(file: HealthFile): void {
  const now = Date.now();
  for (const [key, rec] of Object.entries(file.models)) {
    if (rec.cooldownUntil <= now && rec.status === "cooling") {
      rec.status = "ok";
      rec.cooldownUntil = 0;
    }
    if (rec.status === "ok" && rec.cooldownUntil === 0 && now - (rec.lastFailureAt ?? 0) > 7 * 86_400_000) {
      delete file.models[key];
    }
  }
  for (const [key, rec] of Object.entries(file.owners)) {
    if (rec.cooldownUntil <= now) delete file.owners[key];
  }
}

export function modelKey(provider: string, model: string): string {
  return `${provider}/${model}`;
}

export function ownerKey(provider: string, model: string): string {
  const owner = String(model.split("/")[0] ?? "").toLowerCase();
  return `owner:${owner || provider}`;
}

export function cooling(provider: string, model: string, now = Date.now()): boolean {
  const file = load();
  prune(file);
  return (
    (file.models[modelKey(provider, model)]?.cooldownUntil ?? 0) > now ||
    (file.owners[ownerKey(provider, model)]?.cooldownUntil ?? 0) > now
  );
}

export function cooldownRemaining(provider: string, model: string, now = Date.now()): number {
  const file = load();
  prune(file);
  return Math.max(
    (file.models[modelKey(provider, model)]?.cooldownUntil ?? 0) - now,
    (file.owners[ownerKey(provider, model)]?.cooldownUntil ?? 0) - now,
    0,
  );
}

export function recordSuccess(provider: string, model: string): void {
  const file = load();
  const key = modelKey(provider, model);
  const rec = (file.models[key] ??= {
    provider,
    model,
    status: "ok",
    cooldownUntil: 0,
    successCount: 0,
    failureCount: 0,
  });
  rec.status = "ok";
  rec.cooldownUntil = 0;
  rec.lastSuccessAt = Date.now();
  rec.successCount += 1;
  delete file.owners[ownerKey(provider, model)];
  save(file);
}

export function recordFailure(
  provider: string,
  model: string,
  status: number | undefined,
  opts: { retryAfterMs?: number; ownerWide?: boolean; errorClass?: string } = {},
): void {
  const now = Date.now();
  let ms = 60_000;
  let ownerWide = opts.ownerWide === true;
  if (status === 404 || status === 410) ms = 60 * 60_000;
  else if (status === 401 || status === 403) {
    ms = 60 * 60_000;
    ownerWide = true;
  } else if (status === 429) {
    ms = opts.retryAfterMs ?? 30 * 60_000;
    const owner = String(model.split("/")[0] ?? "").toLowerCase();
    if (owner === "glm" || owner === "zai") ownerWide = true;
  } else if (status && status >= 500) {
    ms = 5 * 60_000;
    ownerWide = true;
  } else if (!status) {
    ms = 60_000;
  }
  const file = load();
  const key = modelKey(provider, model);
  const rec = (file.models[key] ??= {
    provider,
    model,
    status: "cooling",
    cooldownUntil: now + ms,
    successCount: 0,
    failureCount: 0,
  });
  rec.status = "cooling";
  rec.cooldownUntil = now + ms;
  rec.lastFailureAt = now;
  rec.lastStatus = status;
  rec.errorClass = opts.errorClass;
  rec.failureCount += 1;
  if (ownerWide) {
    file.owners[ownerKey(provider, model)] = {
      cooldownUntil: now + ms,
      errorClass: opts.errorClass,
      lastStatus: status,
    };
  }
  save(file);
}

export function recordRetryAfter(provider: string, model: string, headers: Record<string, string> | undefined): number | undefined {
  const hint = headers?.["retry-after"] ?? headers?.["Retry-After"];
  if (!hint) return undefined;
  if (/^\d+$/.test(hint.trim())) return Math.min(24 * 3600_000, Math.max(60_000, Number(hint) * 1000));
  const at = Date.parse(hint);
  return Number.isFinite(at) ? Math.min(24 * 3600_000, Math.max(60_000, at - Date.now())) : undefined;
}

export function healthSnapshot(): HealthFile {
  const file = load();
  prune(file);
  return file;
}
