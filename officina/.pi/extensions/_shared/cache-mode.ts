// Cross-extension cache mode. pi loads every extension through a separate jiti
// instance (moduleCache: false), so the shared state must live on globalThis.

export type CacheModeSetting = "auto" | "local" | "cloud" | "off";

export interface CacheModeState {
  setting: CacheModeSetting;
  provider: string;
  cloud: boolean;
  reason: "local" | "cloud" | "forced-cloud" | "off";
}

const GLOBAL_KEY = "__officinaCacheModeState";

function state(): CacheModeState {
  const g = globalThis as Record<string, unknown>;
  if (!g[GLOBAL_KEY]) {
    g[GLOBAL_KEY] = { setting: "auto", provider: "", cloud: false, reason: "local" };
  }
  return g[GLOBAL_KEY] as CacheModeState;
}

export function cacheModeSetting(env: NodeJS.ProcessEnv = process.env): CacheModeSetting {
  const raw = String(env.OFFICINA_CACHE_MODE ?? "auto").trim().toLowerCase();
  return raw === "local" || raw === "cloud" || raw === "off" ? raw : "auto";
}

export function providerIsLocal(provider: string | undefined): boolean {
  return !provider || provider === "llamacpp";
}

export function resolveCacheMode(
  setting: CacheModeSetting,
  provider: string | undefined,
): CacheModeState {
  const p = String(provider ?? "");
  if (setting === "off") return { setting, provider: p, cloud: false, reason: "off" };
  if (setting === "cloud") return { setting, provider: p, cloud: true, reason: "forced-cloud" };
  if (setting === "local") return { setting, provider: p, cloud: false, reason: "local" };
  return {
    setting,
    provider: p,
    cloud: !providerIsLocal(provider),
    reason: providerIsLocal(provider) ? "local" : "cloud",
  };
}

export function setCacheModeFromProvider(
  provider: string | undefined,
  env: NodeJS.ProcessEnv = process.env,
): CacheModeState {
  const next = resolveCacheMode(cacheModeSetting(env), provider);
  const current = state();
  current.setting = next.setting;
  current.provider = next.provider;
  current.cloud = next.cloud;
  current.reason = next.reason;
  return current;
}

export function getCacheModeState(): CacheModeState {
  return { ...state() };
}

export function isCloudCacheMode(): boolean {
  return state().cloud;
}

export function cacheClearPercent(env: NodeJS.ProcessEnv = process.env): number {
  const parsed = Number(env.OFFICINA_CACHE_CLEAR_PCT ?? 92);
  if (!Number.isFinite(parsed)) return 92;
  return Math.min(100, Math.max(50, Math.round(parsed)));
}

/**
 * True when the tool-result clear pass should be SKIPPED this turn.
 * Provider-independent (2026-11-11): the engine's KV prefix cache reuses
 * only on exact token-prefix continuation, so ANY mid-history stub
 * invalidates reuse from that point — on the local engine that used to
 * mean a growing re-prefill every turn (the old local rule cleared
 * unconditionally). Defer until the context window actually needs the
 * room (>= OFFICINA_CACHE_CLEAR_PCT), on local and cloud alike.
 * OFFICINA_CACHE_FORCE_CLEAR=1 restores the old always-clear behavior.
 */
export function shouldDeferToolClear(
  contextPercent: number | null | undefined,
  env: NodeJS.ProcessEnv = process.env,
): boolean {
  if (env.OFFICINA_CACHE_FORCE_CLEAR === "1") return false;
  if (contextPercent == null) return true;
  return contextPercent < cacheClearPercent(env);
}
