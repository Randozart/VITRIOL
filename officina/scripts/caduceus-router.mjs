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
const STATE_FILE =
  process.env.CADUCEUS_ROUTER_STATE ||
  join(HOME, ".vitriol", "officina", "caduceus-router-state.json");
const REROUTE_LOG = join(dirname(STATE_FILE), "caduceus-router-reroutes.jsonl");
/** The Caduceus panel writes policy here; the proxy follows it (2026-11-16). */
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
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), FIRST_BYTE_MS);
  try {
    const upstream = await fetch(`${GATEWAY}${headers.__path}`, {
      method: headers.__method,
      headers: headers.__forward,
      body: payload,
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
  const candidates = requested
    ? candidateOrder({ requested, fallbacks, state, nowMs: now })
    : [undefined]; // non-model payloads pass straight through

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
      state = park(state, model ?? "?", Date.now() + 60_000, reason);
      saveState(state);
      logReroute({ model, reason, attempt: sessions });
      lastStatus = 0;
      lastText = reason;
      continue;
    }

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

    // Success: stream it through untouched and never retry from here on.
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
    try {
      for await (const chunk of upstream.body) {
        res.write(chunk);
      }
      res.end();
    } catch {
      res.destroy();
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
});

server.listen(PORT, "127.0.0.1", () => {
  const listed = effectiveFallbacks();
  console.log(
    `caduceus-router on :${PORT} → ${GATEWAY} · fallbacks ${listed.length}${
      listed.length ? ` (${listed.join(", ")})` : " (pass-through)"
    } · first-byte ${FIRST_BYTE_MS}ms · prefs ${PREFS_FILE}`,
  );
});
