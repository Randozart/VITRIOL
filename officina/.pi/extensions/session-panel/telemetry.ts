// Session-panel telemetry rows (2026-11-16).
//
// Why: the sidebar showed the local VITRIOL engine to every session, including
// sessions on cloud lanes. These helpers turn THIS session's own facts (the
// route ledger entry it produced, the gateway/lane state, provider health) into
// row PARTS — plain strings the panel colours and truncates — so the rules are
// pure and testable.
//
// PROVENANCE: original Officina work (this repo).

import { readFileSync, statSync } from "node:fs";

export interface RouteEntryFacts {
  provider?: string;
  model?: string;
  tokensIn?: number;
  tokensOut?: number;
  latencyMs?: number;
  costEstimate?: number;
  outcome?: string;
}

export interface HealthFacts {
  models?: Record<
    string,
    { cooldownUntil?: number; failureCount?: number; successCount?: number }
  >;
}

export interface LaneCandidate {
  id: string;
  owner?: string;
  context_window?: number;
  free?: boolean;
}

export interface CaduceusFacts {
  ts?: number;
  gateway_up?: boolean;
  cooling?: number;
  candidates?: LaneCandidate[];
  budget?: { daily_tokens?: number; daily_cap?: number; allowed?: boolean };
}

/** Model-prefixed usage line for a cloud session's own last request. */
export interface ApiRowParts {
  /** Short, sidebar-friendly model label (last path segment). */
  model: string;
  /** The provider/model id as the ledger recorded it. */
  fullModel: string;
  usage: string;
  latency: string;
  cost: string;
  outcome: "ok" | "error";
  note?: string;
}

/** Pool facts for a virtual free-lane (`officina/nimbus`) session. */
export interface LaneRowParts {
  lanes: number;
  cooling: number;
  next: string | null;
  gatewayUp: boolean;
  budget?: string;
}

/** 1234 → "1.2k"; 999 → "999". */
export function fmtTokens(n: number): string {
  const v = Math.max(0, Math.round(n));
  if (v < 1000) return String(v);
  if (v < 1_000_000) return `${(v / 1000).toFixed(1)}k`;
  return `${(v / 1_000_000).toFixed(1)}M`;
}

/** Euro-ish cost with enough digits to see a real charge. */
export function fmtCost(v: number): string {
  const n = Number.isFinite(v) ? Math.max(0, v) : 0;
  if (n === 0) return "free";
  if (n < 0.01) return `€${n.toFixed(4)}`;
  return `€${n.toFixed(2)}`;
}

/** `dahl/MiniMaxAI/MiniMax-M2.7` → `MiniMax-M2.7`; the sidebar is 34 columns. */
/**
 * Sidebar-friendly model label: the short name plus its PROVIDER, so the
 * operator knows who serves which model (2026-11-16: the sidebar and route strip
 * showed only the id's last segment, e.g. "deepseek-v4.1-flash", with no hint
 * that it came from caduceus vs a local engine).
 */
export function shortModel(model: string, max = 22, provider?: string): string {
  const raw = String(model ?? "");
  const last = raw.split("/").filter(Boolean).pop() ?? "";
  const one = last.trim() || raw.trim();
  const short = one.length > max ? `${one.slice(0, max - 1)}…` : one;
  const owner = String(provider ?? "").trim();
  // Skip the suffix when the id already names the provider (caduceus/... ids
  // are owner/model, so the provider is either implicit or redundant).
  if (!owner || raw.includes("/") || short.includes(owner)) return short;
  return `${short} · ${owner}`;
}

function key(provider?: string, model?: string): string {
  return `${provider ?? "?"}/${model ?? "?"}`;
}

/**
 * The row a cloud session earns: its OWN last routed request. Returns null when
 * there is nothing to say — silence beats inventing a number (P4).
 */
export function apiRowParts(
  entry: RouteEntryFacts | null | undefined,
  health: HealthFacts | null,
  now: number,
): ApiRowParts | null {
  if (!entry || !entry.model) return null;
  const usage = `${fmtTokens(entry.tokensIn ?? 0)}→${fmtTokens(entry.tokensOut ?? 0)} tok`;
  const latency = entry.latencyMs
    ? `${(entry.latencyMs / 1000).toFixed(1)}s`
    : "";
  const outcome = entry.outcome === "error" ? "error" : "ok";
  const record = health?.models?.[key(entry.provider, entry.model)];
  let note: string | undefined;
  const cooling = Number(record?.cooldownUntil ?? 0);
  if (cooling > now) note = `cooling ${Math.ceil((cooling - now) / 60_000)}m`;
  else if ((record?.failureCount ?? 0) > 0 && outcome === "error")
    note = "failing";
  return {
    model: shortModel(entry.model),
    fullModel: entry.model,
    usage,
    latency,
    cost: fmtCost(entry.costEstimate ?? 0),
    outcome,
    note,
  };
}

function coolingMs(
  candidate: LaneCandidate,
  health: HealthFacts | null,
  now: number,
): number {
  // Provider-health keys are `<provider>/<model>`; the gateway lane id already
  // carries the owner as its prefix, so look for a `*/<id>` match.
  const models = health?.models ?? {};
  for (const [k, v] of Object.entries(models)) {
    if (k.endsWith(`/${candidate.id}`)) {
      return Math.max(0, Number(v?.cooldownUntil ?? 0) - now);
    }
  }
  return 0;
}

/**
 * The row a `officina/nimbus` session earns: the free-lane pool it actually
 * routes through — how many lanes exist, how many are cooling, and the next one
 * that fits the current context.
 */
export function laneRowParts(
  facts: CaduceusFacts | null,
  ctxTokens: number,
  health: HealthFacts | null,
  now: number,
): LaneRowParts | null {
  const lanes = Array.isArray(facts?.candidates) ? facts.candidates : [];
  if (lanes.length === 0) return null;
  const ctx = Math.max(0, Math.round(ctxTokens));
  const next =
    lanes.find(
      (c) =>
        c.free !== false &&
        (c.context_window ?? 0) >= ctx &&
        coolingMs(c, health, now) === 0,
    ) ?? null;
  const cooling = lanes.filter((c) => coolingMs(c, health, now) > 0).length;
  const budget = facts?.budget;
  const budgetLine =
    budget && Number(budget.daily_cap ?? 0) > 0
      ? `${fmtTokens(budget.daily_tokens ?? 0)}/${fmtTokens(budget.daily_cap ?? 0)} today`
      : undefined;
  return {
    lanes: lanes.length,
    cooling,
    next: next?.id ?? null,
    gatewayUp: facts?.gateway_up !== false,
    budget: budgetLine,
  };
}

interface CachedDoc<T> {
  at: number;
  mtimeMs: number;
  value: T | null;
}

/**
 * Read a JSON facts file with an mtime+TTL cache. The sidebar renders on a
 * 700 ms engine tick, so a 170 KB facts file must never be parsed per frame
 * (P5). Missing/corrupt files yield null, never a throw.
 */
export function cachedJsonFile<T>(
  path: string,
  ttlMs = 2000,
  read: (p: string) => string = (p) => readFileSync(p, "utf8"),
  now: () => number = Date.now,
): T | null {
  const g = globalThis as {
    __officinaJsonCache?: Map<string, CachedDoc<unknown>>;
  };
  g.__officinaJsonCache ??= new Map();
  const at = now();
  let mtimeMs = 0;
  try {
    mtimeMs = statSync(path).mtimeMs;
  } catch {
    return null;
  }
  const hit = g.__officinaJsonCache.get(path) as CachedDoc<T> | undefined;
  if (hit && hit.mtimeMs === mtimeMs && at - hit.at < ttlMs) return hit.value;
  let value: T | null = null;
  try {
    value = JSON.parse(read(path)) as T;
  } catch {
    value = null;
  }
  g.__officinaJsonCache.set(path, { at, mtimeMs, value });
  return value;
}
