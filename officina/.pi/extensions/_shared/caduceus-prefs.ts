// Caduceus routing preferences (2026-11-12, caduceus panel stage 1).
//
// POLICY file — written only by the Rust TUI, read by every router. The
// caduceus extension writes FACTS (caduceus-state.json); this module holds
// only user policy. Free-ness is derived in catalog.ts and never written
// here (plan invariant I3).
//
// Invariants:
//   I1 parity — null/absent prefs leave every pool untouched.
//   I2 policy only filters and reorders; it never invents candidates.
//   I6 corrupt files are ignored (readPrefs returns null) and surfaced by
//      the TUI, never applied partially.

import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export const PREFS_VERSION = 1;

export type RoutingState = "on" | "off";

export interface ProviderPrefs {
  /** Router enablement. "off" excludes the owner from every pool. */
  routing?: RoutingState;
  /** Explicit rank; ranked owners sort first (ascending). */
  rank?: number;
  /** Allow this owner's paid models while free_only is on. */
  allow_paid?: boolean;
}

export interface CaduceusPrefs {
  version: 1;
  free_only?: boolean;
  per_owner?: number;
  max_candidates?: number;
  providers?: Record<string, ProviderPrefs>;
  models?: { pinned?: string[]; denied?: string[] };
}

export interface PrefCandidate {
  id: string;
  owner: string;
  /** Derived by the catalog; never written by the UI. Informational here. */
  free?: boolean;
}

export function prefsPath(): string {
  return (
    process.env.OFFICINA_CADUCEUS_PREFS_FILE ||
    join(homedir(), ".vitriol", "officina", "caduceus-prefs.json")
  );
}

function clampInt(value: unknown, lo: number, hi: number): number | undefined {
  const n = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(n)) return undefined;
  return Math.max(lo, Math.min(hi, Math.trunc(n)));
}

function listOf(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((v) => String(v ?? "").trim()).filter(Boolean);
}

/** Validate and normalize an arbitrary JSON document.
 *  Returns null for absent/corrupt/unsupported documents (I6). */
export function normalizePrefs(raw: unknown): CaduceusPrefs | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const doc = raw as Record<string, unknown>;
  if (doc.version !== PREFS_VERSION) return null;

  const prefs: CaduceusPrefs = { version: PREFS_VERSION };
  if (typeof doc.free_only === "boolean") prefs.free_only = doc.free_only;
  const perOwner = clampInt(doc.per_owner, 1, 32);
  if (perOwner !== undefined) prefs.per_owner = perOwner;
  const max = clampInt(doc.max_candidates, 1, 256);
  if (max !== undefined) prefs.max_candidates = max;

  if (doc.providers && typeof doc.providers === "object" && !Array.isArray(doc.providers)) {
    const providers: Record<string, ProviderPrefs> = {};
    for (const [key, value] of Object.entries(doc.providers as Record<string, unknown>)) {
      const id = String(key ?? "").trim().toLowerCase();
      if (!id || !value || typeof value !== "object" || Array.isArray(value)) continue;
      const row = value as Record<string, unknown>;
      const entry: ProviderPrefs = {};
      if (row.routing === "on" || row.routing === "off") entry.routing = row.routing;
      const rank = clampInt(row.rank, 0, 999);
      if (rank !== undefined) entry.rank = rank;
      if (typeof row.allow_paid === "boolean") entry.allow_paid = row.allow_paid;
      if (Object.keys(entry).length > 0) providers[id] = entry;
    }
    if (Object.keys(providers).length > 0) prefs.providers = providers;
  }

  if (doc.models && typeof doc.models === "object" && !Array.isArray(doc.models)) {
    const models = doc.models as Record<string, unknown>;
    const pinned = listOf(models.pinned);
    const denied = listOf(models.denied);
    if (pinned.length > 0 || denied.length > 0) prefs.models = { pinned, denied };
  }
  return prefs;
}

export function readPrefs(path = prefsPath()): CaduceusPrefs | null {
  if (!existsSync(path)) return null;
  try {
    return normalizePrefs(JSON.parse(readFileSync(path, "utf8")));
  } catch {
    return null;
  }
}

export function providerPrefs(prefs: CaduceusPrefs | null, owner: string): ProviderPrefs {
  return prefs?.providers?.[owner.trim().toLowerCase()] ?? {};
}

/** Owners whose paid models may enter a free-only pool. */
export function allowPaidOwners(prefs: CaduceusPrefs | null): Set<string> {
  const out = new Set<string>();
  for (const [id, p] of Object.entries(prefs?.providers ?? {})) {
    if (p.allow_paid) out.add(id);
  }
  return out;
}

export function isRoutingOff(prefs: CaduceusPrefs | null, owner: string): boolean {
  return providerPrefs(prefs, owner).routing === "off";
}

/** Filter + reorder a candidate pool. Policy only: the pool builder decides
 *  which candidates exist (free_only/allow_paid); this never adds any (I2).
 *  With null prefs the input array is returned unchanged (I1). */
export function applyPrefs<T extends PrefCandidate>(candidates: T[], prefs: CaduceusPrefs | null): T[] {
  if (!prefs) return candidates;

  const denied = new Set(listOf(prefs.models?.denied).map((s) => s.toLowerCase()));
  const pinnedOrder = new Map(
    listOf(prefs.models?.pinned).map((id, i) => [id.toLowerCase(), i] as const),
  );
  const index = new Map(candidates.map((c, i) => [c, i] as const));

  const kept = candidates.filter(
    (c) => !denied.has(c.id.toLowerCase()) && !isRoutingOff(prefs, c.owner),
  );

  const pinned: T[] = [];
  const rest: T[] = [];
  for (const c of kept) {
    if (pinnedOrder.has(c.id.toLowerCase())) pinned.push(c);
    else rest.push(c);
  }
  pinned.sort(
    (a, b) => (pinnedOrder.get(a.id.toLowerCase()) ?? 0) - (pinnedOrder.get(b.id.toLowerCase()) ?? 0),
  );

  // Explicit ranks first (ascending, stable); unranked keep pool order.
  rest.sort((a, b) => {
    const ra = providerPrefs(prefs, a.owner).rank;
    const rb = providerPrefs(prefs, b.owner).rank;
    if (ra !== undefined || rb !== undefined) {
      const va = ra ?? Number.POSITIVE_INFINITY;
      const vb = rb ?? Number.POSITIVE_INFINITY;
      if (va !== vb) return va - vb;
    }
    return (index.get(a) ?? 0) - (index.get(b) ?? 0);
  });

  return [...pinned, ...rest];
}

/** Apply per-owner and total caps. `envCaps` mirrors the historical env
 *  defaults; explicit prefs values win over them. */
export function capCandidates<T extends PrefCandidate>(
  candidates: T[],
  prefs: CaduceusPrefs | null,
  envCaps: { perOwner: number; max: number },
): T[] {
  const perOwner = prefs?.per_owner ?? envCaps.perOwner;
  const max = prefs?.max_candidates ?? envCaps.max;
  const counts = new Map<string, number>();
  const out: T[] = [];
  for (const c of candidates) {
    if (out.length >= max) break;
    const owner = c.owner.trim().toLowerCase();
    const n = counts.get(owner) ?? 0;
    if (n >= perOwner) continue;
    counts.set(owner, n + 1);
    out.push(c);
  }
  return out;
}
