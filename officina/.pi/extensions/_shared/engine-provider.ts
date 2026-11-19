// Engine provider identity (A2, 2026-11-19): the single source of truth for the
// local VITRIOL engine's provider id, in the middle of a rename alias period.
//
// The CANONICAL id is `vitriol` — the name is ours to hold (Tier A: our record),
// not upstream ABI. The historical `llamacpp` spelling is kept as a LEGACY alias
// for one release so that persisted `llamacpp/...` selections and user override
// files that still carry the old key keep resolving. `llama.cpp` is a third
// spelling some registries have used.
//
// RULE: routing logic never hardcodes one spelling. It calls
// `isEngineProvider(id)` so that ANY current or legacy spelling routes as the
// local engine. When the alias period ends, `llamacpp`/`llama.cpp` drop out of
// LEGACY ids and every reference converges on `vitriol` — the deletion is then a
// mechanical shrink of this one file, not a sweep.

/** The canonical engine provider id (what new selections and registrations use). */
export const ENGINE_PROVIDER_ID = "vitriol";

/**
 * Legacy spellings that still name the local engine (the alias period). Each of
 * these, if it ever appears in a stored selection or a user models.json, is
 * treated as the engine — nothing is orphaned by the rename.
 */
export const LEGACY_ENGINE_PROVIDER_IDS: readonly string[] = [
  "llamacpp",
  "llama.cpp",
];

/**
 * Does this provider id name the local VITRIOL engine, in ANY spelling?
 * `null`/`undefined`/"" are not the engine (some callers treat "no provider"
 * as local too — that is a SEPARATE decision made at the call site, not here).
 */
export function isEngineProvider(id: string | null | undefined): boolean {
  if (!id) return false;
  if (id === ENGINE_PROVIDER_ID) return true;
  return (LEGACY_ENGINE_PROVIDER_IDS as string[]).includes(id);
}

/**
 * "No provider selected, or an engine spelling" — the local-engine case. Several
 * routing sites key off exactly this (an empty provider means local by default).
 */
export function isLocalEngine(id: string | null | undefined): boolean {
  return !id || isEngineProvider(id);
}

/**
 * Resolve the engine's registered provider for a registry lookup: try the
 * canonical spelling first, then each legacy alias, so a selection stored under
 * any spelling still finds its model. Returns the first non-undefined, or
 * `fallback` when none resolve.
 */
export function findEngineModel<T>(
  registry: {
    find?: (provider: string, modelId: string) => T | undefined;
  },
  modelId: string,
  fallback?: T,
): T | undefined {
  const find = registry.find;
  if (!find) return fallback;
  const ids = [ENGINE_PROVIDER_ID, ...LEGACY_ENGINE_PROVIDER_IDS];
  for (const provider of ids) {
    const hit = find(provider, modelId);
    if (hit !== undefined) return hit;
  }
  return fallback;
}
