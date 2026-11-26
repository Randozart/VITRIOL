// Pure decision helpers for the Caduceus routing proxy (2026-11-16).
//
// Why a proxy at all: the gateway is a vendor package and has no supported knob
// for "reroute this request to another model until one answers". Everything in
// this file is deliberately side-effect free so the policy can be tested
// without sockets, files or a live gateway.

/** Default port for the proxy (the gateway keeps 20128). */
export const DEFAULT_ROUTER_PORT = 20129;
/** Upper bound on hint-driven parks: one flaky 429 must not bench a lane for
 *  hours (2026-11-21, router-cascade-fix). Env: CADUCEUS_ROUTER_MAX_PARK_S. */
export const DEFAULT_MAX_PARK_S = 900;
/** Placeholder key a no-model request used to park under; never a real lane
 *  (2026-11-21 R1) — dropped wherever it appears. */
export const PHANTOM_LANE_KEY = "?";
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

/**
 * Park duration for a hint-driven failure: the hint clamped to the
 * operator's ceiling (2026-11-21 R2). maxParkMs <= 0 disables the cap.
 */
export function cappedParkMs(hintMs, maxParkMs = DEFAULT_MAX_PARK_S * 1000) {
  if (typeof hintMs !== "number" || !Number.isFinite(hintMs)) return undefined;
  const floor = Math.max(1_000, Math.floor(hintMs));
  if (typeof maxParkMs !== "number" || maxParkMs <= 0) return floor;
  return Math.min(floor, maxParkMs);
}

/** Drop entries whose time has passed (keeps the state file small). */
export function pruneParked(state, nowMs = Date.now()) {
  const next = { parked: {}, reasons: {} };
  for (const [model, until] of Object.entries(state?.parked ?? {})) {
    if (model === PHANTOM_LANE_KEY) continue; // 2026-11-21 R1: never a lane
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
  const soonest = Object.entries(parked ?? {})
    .filter(([model, t]) => model !== PHANTOM_LANE_KEY && typeof t === "number" && t > nowMs)
    .map(([, t]) => t)
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

// ── Decision-model lane choice (2026-11-16) ──────────────────────────────
// The proxy owns one decision the gateway cannot make for us: which model id to
// send. When two candidates tie on the deterministic preference key, the
// decision model breaks the tie — with METADATA ONLY, never the prompt.

/** Token-estimate bucket, so a request's size can be described without sending it. */
export function tokenBucket(tokens) {
  const n = Number(tokens) || 0;
  if (n < 8_000) return "lt8k";
  if (n < 32_000) return "8k_32k";
  if (n < 128_000) return "32k_128k";
  return "128k_plus";
}

/**
 * The Decisions-API request for a lane choice.
 *
 * PRIVACY: the input carries only metadata (requested model, token bucket,
 * streaming flag). Lane criteria carry facts we already publish to /doctor.
 * No prompt text, no output tail, no cassette.
 *
 * @returns {{model: string, input: string, questions: Record<string, unknown>,
 *            lanes: Array<{key: string, model: string}>}}
 */
export function buildLaneDecision({
  requested,
  candidates,
  tokenEstimate,
  streaming = true,
  model = "gemini-decision-1",
}) {
  const lanes = candidates.slice(0, 24).map((c, i) => ({
    key: `lane_${i}`,
    model: String(c.model ?? c.id ?? ""),
    free: typeof c.free === "boolean" ? c.free : null,
    context: Number(c.contextWindow ?? c.context ?? 0),
  }));
  const criteria = {};
  for (const lane of lanes) {
    const cost = lane.free === null ? "cost unknown" : lane.free ? "free" : "paid";
    const context = lane.context > 0 ? `${Math.round(lane.context / 1000)}k ctx` : "context unknown";
    criteria[lane.key] = `${lane.model} (${cost}, ${context})`;
  }
  return {
    model,
    input:
      `Choose the lane for this request. Requested model: ${requested || "none"}. ` +
      `Prompt size: ${tokenBucket(tokenEstimate)}. Streaming: ${streaming ? "yes" : "no"}. ` +
      `Prefer a free lane if cost is known, and prefer a lane that fits the prompt; prefer the requested model when it is offered.`,
    questions: {
      lane: {
        type: "choice",
        instructions:
          "Pick the single best lane for this request. Weigh: does it fit the prompt, is it available, and is it free.",
        criteria,
      },
    },
    lanes,
  };
}

/**
 * Turn a decisions reply into a lane id.
 *
 * Measured rule (2026-11-16): `choice` is NOT the argmax, so the pick comes from
 * the probabilities. Constraints win over the model: a lane from `candidates`
 * only, never one in `exclude` (parked / too small), and below `floor` we keep
 * the deterministic order.
 *
 * @returns {{lane: string|undefined, p: number, reason: string}}
 */
export function pickLaneFromReply(
  replyDoc,
  { request, lanes, exclude = new Set(), floor = 0.6 },
) {
  const answer = replyDoc?.answers?.lane;
  const probs = {};
  for (const [k, v] of Object.entries(answer?.probabilities ?? {})) {
    if (typeof v === "number" && Number.isFinite(v)) probs[k] = v;
  }
  const ranked = Object.entries(probs).sort((a, b) => b[1] - a[1]);
  if (ranked.length === 0)
    return { lane: undefined, p: 0, reason: "no probabilities" };
  for (const [key, p] of ranked) {
    const lane = lanes.find((l) => l.key === key);
    if (!lane) continue;
    if (exclude.has(lane.model)) continue;
    if (p < floor) {
      return { lane: undefined, p, reason: `below floor (p=${p.toFixed(3)})` };
    }
    return { lane: lane.model, p, reason: `decision model p=${p.toFixed(3)}` };
  }
  return {
    lane: undefined,
    p: 0,
    reason: "every ranked lane is excluded (parked or unusable)",
  };
}

/**
 * Should the model be asked at all? Only when the deterministic order is
 * genuinely ambiguous: at least two usable candidates that tie on the preference
 * key (free-ness). A single candidate, or a clear free/paid split, needs no call.
 */
export function needsDecisionModel(candidates, parked = new Set()) {
  const usable = candidates.filter(
    (c) => !parked.has(String(c.model ?? c.id ?? "")),
  );
  if (usable.length < 2) return false;
  const freeness = usable.map((c) => (c.free !== false ? 1 : 0));
  const distinct = new Set(freeness).size;
  return distinct === 1; // all free or all paid: the model can break the tie
}

/**
 * Gemini key from the ascensus secret store (mirrors the TS resolver).
 * Accepts a gemini-NAMED key in any section, a `[gemini]` section, or a bare
 * `api_key` that starts `AIza`.
 */
export function geminiKeyFromSecrets(text) {
  if (typeof text !== "string" || text.length === 0) return "";
  let section = "";
  let named = "";
  let sectioned = "";
  let bare = "";
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const header = /^\[([^\]]+)\]$/.exec(line);
    if (header) {
      section = header[1].trim().toLowerCase();
      continue;
    }
    const m = /^(\w+)\s*=\s*(.+)$/.exec(line);
    if (!m) continue;
    const key = m[1].toLowerCase();
    const value = m[2].trim().replace(/^["']|["']$/g, "");
    if (!value) continue;
    if (!named && (key === "gemini_api_key" || key === "gemini_key"))
      named = value;
    if (
      !sectioned &&
      section === "gemini" &&
      (key === "api_key" || key === "key")
    ) {
      sectioned = value;
    }
    if (
      !bare &&
      section === "" &&
      key === "api_key" &&
      value.startsWith("AIza")
    )
      bare = value;
  }
  return named || sectioned || bare;
}
