// Lane health — gateway-side endpoint marking for the free cloud lanes
// (2026-11-11, owner request). Caduceus already probes its provider
// connections and marks them (`isActive`, `testStatus`, `backoffLevel`);
// this module surfaces those marks to the lane routers so a dead lane is
// skipped BEFORE the first wasted turn, not after.
//
// Classification (pure, unit-tested):
//   hardDown  — isActive === false: the gateway has the connection
//               disabled/deauthorized → its owners are excluded from
//               candidate selection entirely.
//   backoff   — backoffLevel > 0: the gateway is rate-limit-backing-off
//               this connection → lanes are demoted behind healthy ones
//               and flagged in /nimbus, but not removed.
//   active    — everything else.
//
// The fetch is TTL-cached (default 60 s) in a globalThis singleton
// (jiti-safe per AGENTS.md) and degrades to the last known good state —
// a gateway hiccup must never make lanes look dead (fail-open).
//
// Kill switch: OFFICINA_NO_LANE_HEALTH=1 disables the fetch (all owners
// report active; routers keep the reactive provider-health cooling).

import { homedir } from "node:os";
import { join } from "node:path";

/** Vendored gateway API client (same module caduceus/index.ts uses). */
function vendorApiModule(): string {
  return (
    process.env.OFFICINA_CADUCEUS_VENDOR_API ||
    join(homedir(), ".vitriol", "vendor", "omniroute", "node_modules", "omniroute", "bin", "cli", "api.mjs")
  );
}

export const DEFAULT_TTL_MS = 60_000;

export interface GatewayConnection {
  provider?: string;
  isActive?: boolean;
  testStatus?: string;
  backoffLevel?: number;
  lastTested?: string;
}

export interface LaneHealth {
  /** Owners the gateway has marked disabled/deauthorized. Excluded. */
  hardDown: Set<string>;
  /** Owners the gateway is backing off. Demoted, not excluded. */
  backoff: Set<string>;
  /** Everything else. */
  active: Set<string>;
  checkedAt: number;
}

/**
 * Classify gateway connections into lane-health buckets. Pure. Unknown
 * owners never appear (callers treat "not in any set" as active, which
 * keeps fallback owners that lack a gateway connection alive).
 */
export function classifyConnections(connections: GatewayConnection[]): LaneHealth {
  const out: LaneHealth = {
    hardDown: new Set(),
    backoff: new Set(),
    active: new Set(),
    checkedAt: Date.now(),
  };
  for (const c of connections ?? []) {
    if (typeof c !== "object" || c === null) continue;
    const owner = String(c.provider ?? "").trim().toLowerCase();
    if (!owner) continue;
    if (c.isActive === false) {
      out.hardDown.add(owner);
      continue;
    }
    if (Number(c.backoffLevel ?? 0) > 0) {
      out.backoff.add(owner);
      continue;
    }
    out.active.add(owner);
  }
  return out;
}

export const EMPTY_HEALTH = (): LaneHealth => ({
  hardDown: new Set(),
  backoff: new Set(),
  active: new Set(),
  checkedAt: 0,
});

interface Cache {
  health: LaneHealth;
  at: number;
  /** In-flight promise so concurrent route calls share one fetch. */
  pending: Promise<LaneHealth> | null;
}

function shared(): Cache {
  const g = globalThis as { __officinaLaneHealth?: Cache };
  // globalThis-backed: pi loads each extension through a separate jiti
  // instance (AGENTS.md rule) — module state would not be shared.
  if (!g.__officinaLaneHealth) {
    g.__officinaLaneHealth = { health: EMPTY_HEALTH(), at: 0, pending: null };
  }
  return g.__officinaLaneHealth;
}

async function fetchConnections(): Promise<GatewayConnection[]> {
  const mod = await import(`file://${vendorApiModule()}`);
  const res = await mod.apiFetch("/api/providers?limit=5000", {
    baseUrl: (process.env.OFFICINA_CADUCEUS_BASE_URL || "http://127.0.0.1:20128/v1").replace(/\/v1\/?$/, ""),
    timeout: 5_000,
  });
  if (!res.ok) throw new Error(`gateway HTTP ${res.status}`);
  const body = await res.json();
  const list = Array.isArray(body) ? body : body?.connections ?? [];
  return Array.isArray(list) ? (list as GatewayConnection[]) : [];
}

export function resetLaneHealthCache(): void {
  delete (globalThis as { __officinaLaneHealth?: Cache }).__officinaLaneHealth;
}

/** One selectable lane, as the /nimbus table renders it. */
export interface LaneRow {
  id: string;
  owner: string;
  contextWindow: number;
  /** Remaining provider-health cooldown in ms (0 = healthy). */
  coolingMs: number;
}

/**
 * Render the lane table for /nimbus. Preference order: healthy lanes keep
 * pool order, gateway-backoff lanes follow, cooling lanes next, and
 * gateway-hard-down lanes are listed last with an explicit mark (they are
 * excluded from selection but the operator should still see them).
 * Pure.
 */
export function formatLanes(rows: LaneRow[], health: LaneHealth, now: number): string {
  const rank = (r: LaneRow): number => {
    if (health.hardDown.has(r.owner)) return 3;
    if (health.backoff.has(r.owner)) return 1;
    if (r.coolingMs > 0) return 2;
    return 0;
  };
  const order = rows
    .map((r, i) => ({ r, i }))
    .sort((a, b) => rank(a.r) - rank(b.r) || a.i - b.i);
  const lines: string[] = [];
  for (const { r } of order) {
    const ctx = `${Math.round(r.contextWindow / 1000)}k`;
    const marks: string[] = [];
    if (health.hardDown.has(r.owner)) marks.push("gateway-down");
    else if (health.backoff.has(r.owner)) marks.push("gateway-backoff");
    if (r.coolingMs > 0) marks.push(`cooling ${Math.ceil(r.coolingMs / 60_000)}m`);
    const tag = marks.length ? ` · ${marks.join(" · ")}` : "";
    lines.push(`${r.id} · ${ctx}${tag}`);
  }
  return lines.join("\n");
}

/**
 * Gateway lane health, TTL-cached. Fail-open: on a fetch error the last
 * known good state is kept (and re-dated so the cache is not hammered);
 * with no prior state, everything reports active.
 */
export async function getLaneHealth(
  opts: { now?: number; ttlMs?: number } = {},
): Promise<LaneHealth> {
  const now = opts.now ?? Date.now();
  const ttl = opts.ttlMs ?? (Number(process.env.OFFICINA_LANE_HEALTH_TTL_MS) || DEFAULT_TTL_MS);
  if (process.env.OFFICINA_NO_LANE_HEALTH === "1") {
    return { ...EMPTY_HEALTH(), checkedAt: now };
  }
  const cache = shared();
  if (cache.pending) return cache.pending;
  const fresh = cache.health.checkedAt > 0 && now - cache.health.checkedAt < ttl;
  if (fresh) return cache.health;
  const pending = (async (): Promise<LaneHealth> => {
    try {
      const health = classifyConnections(await fetchConnections());
      cache.health = health;
      cache.at = Date.now();
      return health;
    } catch {
      // Fail-open: keep the previous state, treat as fresh for one TTL.
      cache.at = Date.now();
      return cache.health;
    } finally {
      cache.pending = null;
    }
  })();
  cache.pending = pending;
  return pending;
}
