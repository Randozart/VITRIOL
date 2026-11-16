// Telemetry scope (2026-11-16): which facts describe THIS session's work.
//
// Why: every VITRIOL sidebar row rendered unconditionally, so a session on a
// cloud lane showed the local engine's tok/s and "decoded this boot" numbers —
// someone else's workshop. Scope is derived from the session's own provider and
// drives DISPLAY only: it never changes routing, the model or the agent.
//
// PROVENANCE: original Officina work (this repo).

export type TelemetryScope = "engine" | "lanes" | "cloud" | "none";

/** Providers served by the local VITRIOL engine. */
const ENGINE_PROVIDERS = new Set(["llamacpp", "llama.cpp", "vitriol"]);

/**
 * Which telemetry this session should show.
 *
 * - `llamacpp` (and engine aliases) → `engine`
 * - `officina/lapis` (local-first, cloud overflow) → `engine`
 * - `officina/nimbus` (free-lane pool) → `lanes`
 * - any other provider (caduceus, opencode-go, built-ins) → `cloud`
 * - no provider known yet → `none` (silence beats a foreign number)
 */
export function telemetryScope(
  provider?: string,
  modelId?: string,
): TelemetryScope {
  const p = String(provider ?? "")
    .trim()
    .toLowerCase();
  const m = String(modelId ?? "")
    .trim()
    .toLowerCase();
  if (!p) return "none";
  if (ENGINE_PROVIDERS.has(p)) return "engine";
  if (p === "officina") {
    if (m === "nimbus") return "lanes";
    if (m === "lapis") return "engine";
    return "cloud";
  }
  return "cloud";
}

/** True only for the scope that owns the VITRIOL engine rows. */
export function engineRowsVisible(scope: TelemetryScope): boolean {
  return scope === "engine";
}
