#!/usr/bin/env node
// Caduceus routing proxy (2026-11-16).
//
// Sits in front of the OmniRoute gateway and answers one question: "the lane
// you asked for is quota-spent or dead — is there another one?" It walks
// candidates, parks what fails for the duration the gateway states, and only
// gives up when nothing is left. It never retries after it has started
// forwarding bytes, so a partial answer is never duplicated.
//
// Off unless CADUCEUS_ROUTER=1 (or the unit starts it). With no fallbacks
// configured it is a pure pass-through: same bytes, one extra loopback hop.
//
// Usage: node caduceus-router.mjs [--port N] [--gateway URL]
// Env: CADUCEUS_ROUTER_FALLBACKS=a/b,c/d   ordered fallback model ids
//      CADUCEUS_ROUTER_FIRST_BYTE_MS=45000 per-attempt deadline
//      CADUCEUS_ROUTER_STATE=<path>        parked-lane state file

import { createServer } from "node:http";
import {
  appendFileSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import {
  DEFAULT_ROUTER_PORT,
  candidateOrder,
  giveUpMessage,
  park,
  parseQuotaHint,
  pruneParked,
  resolveFallbacks,
  buildLaneDecision,
  needsDecisionModel,
  pickLaneFromReply,
  tokenBucket,
  isParked,
  geminiKeyFromSecrets,
} from "./caduceus-router-lib.mjs";

const HOME = process.env.HOME || homedir();
const argv = process.argv.slice(2);
const argOf = (flag, fallback) => {
  const i = argv.indexOf(flag);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
};
const PORT = Number(
  argOf("--port", process.env.CADUCEUS_ROUTER_PORT || DEFAULT_ROUTER_PORT),
);
const GATEWAY = argOf(
  "--gateway",
  process.env.CADUCEUS_GATEWAY_URL || "http://127.0.0.1:20128",
).replace(/\/+$/, "");
const FIRST_BYTE_MS = Math.max(
  5_000,
  Number(process.env.CADUCEUS_ROUTER_FIRST_BYTE_MS) || 45_000,
);
const DECISION_ON = /^(1|on|true|yes)$/i.test(
  String(argOf("--decision", process.env.CADUCEUS_ROUTER_DECISION || "0")),
);
const DECISION_URL = argOf(
  "--decision-url",
  process.env.CADUCEUS_ROUTER_DECISION_URL ||
    "https://generativelanguage.googleapis.com/v1beta/decisions",
);
const DECISION_MODEL = argOf(
  "--decision-model",
  process.env.CADUCEUS_ROUTER_DECISION_MODEL || "gemini-decision-1",
);
const DECISION_FLOOR =
  Number(
    argOf(
      "--decision-floor",
      process.env.CADUCEUS_ROUTER_DECISION_FLOOR || "0.6",
    ),
  ) || 0.6;
const DECISION_TIMEOUT_MS = 2_500;
/** One call per (requested, bucket, lane set) per window (2026-11-16). */
const DECISION_MEMO_MS = 30_000;
let decisionMemo = { key: "", at: 0, lane: "" };
/** Context windows, cached: the lane criteria want them, not the prompt. */
let modelCache = { at: 0, byId: new Map() };

/** Gemini key: env, then the ascensus secret store. */
function geminiKey() {
  const env = String(process.env.GEMINI_API_KEY ?? "").trim();
  if (env) return env;
  try {
    return geminiKeyFromSecrets(readFileSync(SECRETS_FILE, "utf8"));
  } catch {
    return "";
  }
}

/** Context windows for the candidate lanes (cached 60 s, failures ignored). */
async function laneFacts() {
  if (Date.now() - modelCache.at < 60_000 && modelCache.byId.size > 0) {
    return modelCache.byId;
  }
  try {
    const res = await fetch(`${GATEWAY}/v1/models`, {
      headers: {
        authorization: `Bearer ${process.env.OFFICINA_CADUCEUS_API_KEY ?? ""}`,
      },
      signal: AbortSignal.timeout(2_000),
    });
    if (!res.ok) return modelCache.byId;
    const doc = await res.json();
    const byId = new Map();
    for (const m of doc?.data ?? []) {
      if (m?.id)
        byId.set(
          String(m.id),
          Number(m.context_length ?? m.context_window ?? 0) || 0,
        );
    }
    modelCache = { at: Date.now(), byId };
    return byId;
  } catch {
    return modelCache.byId;
  }
}

/**
 * Ask the decision model which lane to use, when — and only when — the
 * deterministic order is ambiguous. Metadata only; any failure returns
 * undefined and the proxy keeps its own order (R-A5).
 */
async function decideLane(requested, candidates, parked, tokenEstimate) {
  if (!DECISION_ON) return undefined;
  if (!needsDecisionModel(candidates, parked)) return undefined;
  const key = geminiKey();
  if (!key) return undefined;
  const memoKey = `${requested}|${tokenBucket(tokenEstimate)}|${candidates.join(",")}`;
  if (
    decisionMemo.key === memoKey &&
    Date.now() - decisionMemo.at < DECISION_MEMO_MS
  ) {
    return decisionMemo.lane || undefined;
  }
  const facts = await laneFacts();
  const built = buildLaneDecision({
    requested,
    candidates: candidates.map((id) => ({
      model: id,
      free: true,
      contextWindow: facts.get(id) ?? 0,
    })),
    tokenEstimate,
    model: DECISION_MODEL,
  });
  let lane;
  try {
    const res = await fetch(DECISION_URL, {
      method: "POST",
      headers: { "content-type": "application/json", "x-goog-api-key": key },
      body: JSON.stringify({
        model: built.model,
        input: built.input,
        questions: built.questions,
      }),
      signal: AbortSignal.timeout(DECISION_TIMEOUT_MS),
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const doc = await res.json();
    const picked = pickLaneFromReply(doc, {
      lanes: built.lanes,
      exclude: parked,
      floor: DECISION_FLOOR,
    });
    lane = picked.lane;
    if (lane)
      logReroute({ model: lane, decision: true, reason: picked.reason });
    else
      logReroute({ model: requested, decision: true, reason: picked.reason });
  } catch (error) {
    logReroute({
      model: requested,
      decision: true,
      reason: `decision failed: ${String(error?.message ?? error)}`,
    });
    lane = undefined;
  }
  decisionMemo = { key: memoKey, at: Date.now(), lane: lane ?? "" };
  return lane;
}

const STATE_FILE =
  process.env.CADUCEUS_ROUTER_STATE ||
  join(HOME, ".vitriol", "officina", "caduceus-router-state.json");
const REROUTE_LOG = join(dirname(STATE_FILE), "caduceus-router-reroutes.jsonl");
/** The Caduceus panel writes policy here; the proxy follows it (2026-11-16). */
const SECRETS_FILE = join(HOME, ".vitriol", "secrets");
const PREFS_FILE =
  process.env.OFFICINA_CADUCEUS_PREFS_FILE ||
  join(HOME, ".vitriol", "officina", "caduceus-prefs.json");

/** Parked lanes, persisted so a restart does not resurrect a spent lane. */
function loadState() {
  try {
    return pruneParked(JSON.parse(readFileSync(STATE_FILE, "utf8")));
  } catch {
    return { parked: {}, reasons: {} };
  }
}

function saveState(state) {
  try {
    mkdirSync(dirname(STATE_FILE), { recursive: true });
    const tmp = `${STATE_FILE}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(pruneParked(state), null, 2)}\n`, {
      mode: 0o600,
    });
    renameSync(tmp, STATE_FILE);
  } catch {
    /* the proxy must keep serving even if the state file is unwritable */
  }
}

/** Prefs cache keyed on mtime: the panel can change fallbacks while we run. */
let prefsCache = { mtime: -1, doc: null };

function readPrefs() {
  try {
    const info = statSync(PREFS_FILE);
    const mtime = info.mtimeMs;
    if (mtime !== prefsCache.mtime) {
      prefsCache = { mtime, doc: JSON.parse(readFileSync(PREFS_FILE, "utf8")) };
    }
    return prefsCache.doc;
  } catch (error) {
    console.error(`[prefs] ${PREFS_FILE}: ${error?.message ?? error}`);
    return null;
  }
}

/** Effective fallbacks for one request (env wins over the panel's prefs). */
function effectiveFallbacks() {
  return resolveFallbacks({ env: process.env, prefs: readPrefs() });
}

function logReroute(entry) {
  try {
    mkdirSync(dirname(REROUTE_LOG), { recursive: true });
    appendFileSync(
      REROUTE_LOG,
      `${JSON.stringify({ ts: new Date().toISOString(), ...entry })}\n`,
    );
  } catch {
    /* decoration only */
  }
}

let state = loadState();

/** Read the whole request body (the proxy needs the model id). */
async function readBody(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  return Buffer.concat(chunks);
}

/** Extract `model` from a JSON body, or undefined for non-JSON payloads. */
function modelOf(body) {
  try {
    const parsed = JSON.parse(body.toString("utf8"));
    return typeof parsed?.model === "string" ? parsed.model : undefined;
  } catch {
    return undefined;
  }
}

/** One attempt against the gateway. Resolves with the upstream Response. */
async function attempt(model, body, headers) {
  const payload = model
    ? Buffer.from(
        JSON.stringify({ ...JSON.parse(body.toString("utf8")), model }),
      )
    : body;
  // GET/HEAD cannot carry a body (catalog reads, health probes): the proxy is a
  // general reverse proxy, not just a chat endpoint (2026-11-16).
  const method = String(headers.__method ?? "POST").toUpperCase();
  const sendBody = method !== "GET" && method !== "HEAD" && payload.length > 0;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), FIRST_BYTE_MS);
  try {
    const upstream = await fetch(`${GATEWAY}${headers.__path}`, {
      method,
      headers: headers.__forward,
      ...(sendBody ? { body: payload } : {}),
      signal: controller.signal,
    });
    return { upstream, timer, controller };
  } catch (error) {
    clearTimeout(timer);
    throw error;
  }
}

const server = createServer(async (req, res) => {
  if (req.method === "GET" && req.url === "/health") {
    const now = Date.now();
    const parked = Object.entries(state.parked ?? {}).filter(
      ([, until]) => until > now,
    );
    res.writeHead(200, { "content-type": "application/json" });
    res.end(
      JSON.stringify({
        ok: true,
        gateway: GATEWAY,
        fallbacks: effectiveFallbacks(),
        parked: parked.map(([model, until]) => ({
          model,
          seconds: Math.round((until - now) / 1000),
        })),
      }),
    );
    return;
  }

  const body = await readBody(req);
  const requested = modelOf(body);
  const fallbacks = effectiveFallbacks();
  const now = Date.now();
  state = pruneParked(state, now);
  let candidates = requested
    ? candidateOrder({ requested, fallbacks, state, nowMs: now })
    : [undefined]; // non-model payloads pass straight through

  // Decision tier (2026-11-16): when the deterministic order is ambiguous, ask
  // the decision model which lane to try FIRST. It can only reorder — the walk
  // below still falls through on failure, so this never costs a request.
  if (requested && DECISION_ON) {
    const parked = new Set(
      Object.keys(state.parked ?? {}).filter((m) => isParked(state, m, now)),
    );
    const lane = await decideLane(
      requested,
      candidates,
      parked,
      Math.ceil(body.length / 4),
    );
    // R-A1: the model may reorder the FALLBACKS, never the requested model —
    // that id is what the client (and possibly the operator's pin) asked for.
    // So a pick only moves to the front of the tail.
    if (lane && candidates.includes(lane)) {
      const head = candidates[0] === requested ? [requested] : [];
      const tail = candidates.filter((c) => !head.includes(c) && c !== lane);
      const next = [...head, lane, ...tail];
      if (next.join(",") !== candidates.join(",")) candidates = next;
    }
  }

  if (candidates.length === 0) {
    const message = giveUpMessage({
      tried: fallbacks.length + 1,
      parked: state.parked,
      nowMs: now,
    });
    res.writeHead(503, { "content-type": "application/json" });
    res.end(
      JSON.stringify({
        error: { message, type: "router_unavailable", code: 503 },
      }),
    );
    return;
  }

  const forward = { ...req.headers };
  delete forward.host;
  delete forward["content-length"];
  const meta = { __path: req.url, __method: req.method, __forward: forward };

  return serveRequest(res, meta, body, candidates);
});

/**
 * The candidate walk (2026-11-20): extracted from the request handler so the
 * mid-stream recovery can RECURSE with the remaining candidates (pre-first-
 * byte death is recoverable — nothing has hit the client wire yet). `meta`
 * carries the already-built forward headers; `body` the buffered request.
 */
async function serveRequest(res, meta, body, candidates) {
  let sessions = 0; // one session per request: a reused tracker, not a counter
  let lastStatus = 0;
  let lastText = "";

  for (const model of candidates) {
    sessions += 1;
    let upstream;
    let timer;
    try {
      const result = await attempt(model, body, meta);
      upstream = result.upstream;
      timer = result.timer;
    } catch (error) {
      clearTimeout(timer);
      const reason =
        error?.name === "AbortError"
          ? "first-byte timeout"
          : String(error?.message ?? error);
      // 2026-11-20 (S4): transient vs fatal. A network-level reset (fetch
      // TypeError: ECONNRESET/terminated, no HTTP response at all) says
      // nothing about the LANE — parking it 60s benches healthy capacity and
      // starves the walk. Only failures WITH a response (429/5xx, hint from
      // the gateway) or first-byte timeouts park. Transient resets just move
      // to the next candidate.
      const transient = error?.name !== "AbortError" && !error?.status;
      if (!transient) {
        state = park(state, model ?? "?", Date.now() + 60_000, reason);
        saveState(state);
      }
      logReroute({ model, reason, transient, attempt: sessions });
      lastStatus = 0;
      lastText = reason;
      continue;
    }

    // 2026-11-20 (S2): the success path NEVER cleared the 45s first-byte
    // timer, so controller.abort() fired at T+45s and killed healthy
    // mid-generation streams. From here on the deadline is irrelevant —
    // first byte has arrived; disown the timer entirely.
    clearTimeout(timer);
    lastStatus = upstream.status;

    if (upstream.status >= 400) {
      // Nothing has been forwarded yet — safe to read the error and reroute.
      lastText = await upstream.text().catch(() => "");
      const hint = parseQuotaHint(lastText, Date.now());
      state = park(
        state,
        model ?? "?",
        Date.now() + (hint ?? 60_000),
        `HTTP ${upstream.status}`,
      );
      saveState(state);
      logReroute({
        model,
        status: upstream.status,
        parkedSeconds: Math.round((hint ?? 60_000) / 1000),
        attempt: sessions,
      });
      continue; // next candidate; the last failure is reported if none left
    }

    // Success: stream it through. 2026-11-20 (S3): "never retry" was the
    // defect — a lane that accepted the request and then died mid-stream
    // surfaced to the client as a dead connection. Two windows:
    //   pre-first-byte  — safe to reroute (nothing written to the client);
    //   post-first-byte — unrecoverable (I5), so emit a well-formed SSE
    //                    terminal error instead of res.destroy() hang.
    const outHeaders = {};
    for (const [k, v] of upstream.headers) {
      if (k === "content-length" || k === "transfer-encoding") continue;
      outHeaders[k] = v;
    }
    outHeaders["x-caduceus-router"] = model
      ? `model=${model};attempt=${sessions}`
      : `attempt=${sessions}`;
    if (sessions > 1)
      logReroute({
        model,
        status: upstream.status,
        served: true,
        attempt: sessions,
      });
    res.writeHead(upstream.status, outHeaders);
    let wrote = 0;
    try {
      for await (const chunk of upstream.body) {
        res.write(chunk);
        wrote += 1;
      }
      res.end();
    } catch (error) {
      if (wrote === 0) {
        // Pre-first-byte death: the walk can still recover. Reroute WITHOUT
        // parking (transient, S4) — recurse with the remaining candidates.
        logReroute({
          model,
          reason: `mid-stream reset before first chunk: ${String(error?.message ?? error)}`,
          attempt: sessions,
        });
        return serveRequest(req, res, meta, body, candidates.slice(sessions));
      }
      // Post-first-byte death (I5): never reroute after bytes hit the wire.
      // Close the stream cleanly with a terminal SSE error event so the
      // client sees a definitive end, not a hang.
      logReroute({
        model,
        reason: `mid-stream reset after ${wrote} chunk(s)`,
        attempt: sessions,
      });
      const tail = `\n\nevent: error\ndata: ${JSON.stringify({
        error: {
          message: `upstream lane died mid-stream (${model ?? "gateway"})`,
          type: "caduceus_midstream_failure",
          code: 502,
        },
      })}\n\n`;
      try {
        res.write(tail);
        res.end();
      } catch {
        res.destroy();
      }
    }
    return;
  }

  // Every candidate failed (or all were parked).
  const message =
    lastStatus === 0
      ? `all ${sessions} candidate(s) failed — last: ${lastText}`
      : lastText || `all ${sessions} candidate(s) failed`;
  res.writeHead(lastStatus >= 400 ? lastStatus : 502, {
    "content-type": "application/json",
  });
  res.end(
    JSON.stringify({
      error: { message, type: "router_exhausted", code: lastStatus || 502 },
    }),
  );
}

server.listen(PORT, "127.0.0.1", () => {
  const listed = effectiveFallbacks();
  console.log(
    `caduceus-router on :${PORT} → ${GATEWAY} · fallbacks ${listed.length}${
      listed.length ? ` (${listed.join(", ")})` : " (pass-through)"
    } · first-byte ${FIRST_BYTE_MS}ms · prefs ${PREFS_FILE}`,
  );
});
