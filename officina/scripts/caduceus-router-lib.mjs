// Pure decision helpers for the Caduceus routing proxy (2026-11-16).
//
// Why a proxy at all: the gateway is a vendor package and has no supported knob
// for "reroute this request to another model until one answers". Everything in
// this file is deliberately side-effect free so the policy can be tested
// without sockets, files or a live gateway.

/** Default port for the proxy (the gateway keeps 20128). */
export const DEFAULT_ROUTER_PORT = 20129;
/** Below this, don't bother retrying: the request was answered. */
export const RETRYABLE_STATUSES = new Set([
  401, 403, 404, 408, 410, 429, 500, 502, 503, 504, 529,
]);

/**
 * How long the gateway asked us to wait, from an error text. Accepts the shapes
 * Caduceus actually emits: `"retry_after":177` (seconds, possibly quoted) and
 * `"reset_at":"2026-11-16T18:54:18.506Z"`. Clamped 1 s … 24 h.
 *
 * @returns {number|undefined} milliseconds
 */
export function parseQuotaHint(text, nowMs = Date.now()) {
  if (typeof text !== "string" || text.length === 0) return undefined;
  const clamp = (ms) =>
    Math.min(24 * 3600_000, Math.max(1_000, Math.floor(ms)));
  const seconds = /"retry_after"\s*:\s*"?([0-9]+(?:\.[0-9]+)?)"?/.exec(text);
  if (seconds) return clamp(Number(seconds[1]) * 1000);
  const iso = /"(?:reset_at|retry_after)"\s*:\s*"([^"]+)"/.exec(text);
  if (iso) {
    const at = Date.parse(iso[1]);
    if (Number.isFinite(at)) return clamp(at - nowMs);
  }
  return undefined;
}

/** Is this lane parked right now? */
export function isParked(state, model, nowMs = Date.now()) {
  const until = state?.parked?.[model];
  return typeof until === "number" && until > nowMs;
}

/** Park `model` until `untilMs`, keeping the reason for the operator. */
export function park(state, model, untilMs, reason) {
  const next = { ...(state ?? { parked: {} }) };
  next.parked = { ...(next.parked ?? {}) };
  next.parked[model] = untilMs;
  next.reasons = { ...(next.reasons ?? {}), [model]: reason ?? "" };
  return next;
}

/** Drop entries whose time has passed (keeps the state file small). */
export function pruneParked(state, nowMs = Date.now()) {
  const next = { parked: {}, reasons: {} };
  for (const [model, until] of Object.entries(state?.parked ?? {})) {
    if (typeof until === "number" && until > nowMs) {
      next.parked[model] = until;
      next.reasons[model] = state?.reasons?.[model] ?? "";
    }
  }
  return next;
}

/**
 * Ordered candidates for one request: the requested model first (a pinned
 * choice is honoured first, always), then configured fallbacks that are not
 * parked. Parked lanes are never returned, so a quota-spent model cannot be
 * re-picked.
 */
export function candidateOrder({
  requested,
  fallbacks = [],
  state,
  nowMs = Date.now(),
}) {
  const out = [];
  const push = (model) => {
    const id = typeof model === "string" ? model.trim() : "";
    if (id.length === 0) return;
    if (out.includes(id)) return;
    if (isParked(state, id, nowMs)) return;
    out.push(id);
  };
  push(requested);
  for (const model of fallbacks) push(model);
  return out;
}

/**
 * Should this attempt be retried on the next candidate? Only when nothing has
 * been forwarded to the client yet — a partially streamed answer must never be
 * replayed (I35).
 */
export function shouldRetry({ status, forwarded, timedOut }) {
  if (forwarded) return false;
  if (timedOut) return true;
  return typeof status === "number" && RETRYABLE_STATUSES.has(status);
}

/** One line for the operator when every candidate is parked or failed. */
export function giveUpMessage({ tried, parked, nowMs = Date.now() }) {
  const soonest = Object.values(parked ?? {})
    .filter((t) => typeof t === "number" && t > nowMs)
    .sort((a, b) => a - b)[0];
  const when = soonest
    ? `earliest lane returns in ${Math.max(1, Math.round((soonest - nowMs) / 1000))}s`
    : "no lane is parked";
  return `all ${tried} candidate(s) unavailable — ${when}`;
}

/** Fallbacks from the Caduceus prefs document (`fallback: [...]`, 2026-11-16). */
export function fallbacksFromPrefs(prefs) {
  const raw = prefs?.fallback;
  if (!Array.isArray(raw)) return [];
  return raw.map((m) => String(m ?? "").trim()).filter((m) => m.length > 0);
}

/**
 * Effective fallbacks: the env wins (so a unit can pin them), otherwise the
 * panel's policy file decides. The Caduceus panel is the normal way to set
 * this; env exists for scripted setups.
 */
export function resolveFallbacks({ env = process.env, prefs = null } = {}) {
  const fromEnv = fallbacksFromEnv(env);
  return fromEnv.length > 0 ? fromEnv : fallbacksFromPrefs(prefs);
}

/** Fallback list from env (`CADUCEUS_ROUTER_FALLBACKS`, comma separated). */
export function fallbacksFromEnv(env = process.env) {
  return String(env.CADUCEUS_ROUTER_FALLBACKS ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}
