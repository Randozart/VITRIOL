// Cloud probe (2026-11-14) — background admission service.
//
// Operator: "Caduceus probes the cloud in the background, and keeps trying,
// unless truly everything is exhausted."
//
// One leader session (lease in the shared facts file) probes candidate lanes
// with a 1-token "ping" and publishes the outcome through the SAME parsers and
// marks as real failures, so every router sees cooled/paused/invalid lanes
// BEFORE spending a turn on them. Probes never carry session content.
//
// Kill switch: OFFICINA_NO_PROBE=1.
// Tuning: OFFICINA_PROBE_PER_MIN (6), OFFICINA_PROBE_DAILY (2000),
//         OFFICINA_PROBE_HEARTBEAT_MS (10 min), OFFICINA_PROBE_TICK_MS (15 s),
//         OFFICINA_PROBE_VERIFIED_TTL_MS (10 min).

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  acquireProbeLease,
  clearOwnerPause,
  markGatewayInvalid,
  markModelLimit,
  markOffered,
  markOwnerPause,
  markProbe,
  ownerOfLane,
  parseContextLimit,
  parseCooldownFromError,
  parseModelNotFound,
  parseOwnerPause,
  readGatewayMarks,
  releaseProbeLease,
  type ProbeFact,
} from "../_shared/gateway-health.ts";
import {
  emptyBucket,
  isExhausted,
  probeBackoffMs,
  rateAllow,
  selectProbeCandidate,
  type ProbeLane,
  type ProbeWorld,
  type RateBucket,
} from "../_shared/probe-scheduler.ts";
import { healthSnapshot, recordFailure, recordSuccess } from "../_shared/provider-health.ts";

const TICK_MS = Number(process.env.OFFICINA_PROBE_TICK_MS) || 15_000;
const HEARTBEAT_MS = Number(process.env.OFFICINA_PROBE_HEARTBEAT_MS) || 10 * 60_000;
const PER_MIN = Number(process.env.OFFICINA_PROBE_PER_MIN) || 6;
const DAILY = Number(process.env.OFFICINA_PROBE_DAILY) || 2000;
const VERIFIED_TTL_MS = Number(process.env.OFFICINA_PROBE_VERIFIED_TTL_MS) || 10 * 60_000;
const LEASE_TTL_MS = 20_000;
const PROBE_TIMEOUT_MS = 20_000;

function envConfig(): { baseUrl: string; key: string } {
  const base = String(process.env.OFFICINA_CADUCEUS_BASE_URL || "http://127.0.0.1:20128/v1").replace(
    /\/+$/,
    "",
  );
  return { baseUrl: base, key: String(process.env.OFFICINA_CADUCEUS_API_KEY ?? "").trim() };
}

function loadLanes(): ProbeLane[] {
  try {
    const path = join(homedir(), ".vitriol", "officina", "caduceus-state.json");
    const doc = JSON.parse(readFileSync(path, "utf8")) as {
      candidates?: Array<{ id?: string; owner?: string; free?: boolean }>;
    };
    const lanes: ProbeLane[] = [];
    for (const c of doc.candidates ?? []) {
      const id = String(c.id ?? "").trim();
      if (!id || c.free === false) continue;
      lanes.push({ id, owner: String(c.owner ?? ownerOfLane(id)).trim().toLowerCase() });
    }
    return lanes;
  } catch {
    return [];
  }
}

function coolingMap(): Record<string, number> {
  const out: Record<string, number> = {};
  try {
    const snap = healthSnapshot();
    for (const [key, rec] of Object.entries(snap.models ?? {})) {
      const until = Number(rec?.cooldownUntil ?? 0);
      if (until > 0) {
        // key is "caduceus/<lane>"
        const lane = key.includes("/") ? key.split("/").slice(1).join("/") : key;
        out[lane] = until;
      }
    }
  } catch {
    // no health file yet
  }
  return out;
}

function world(): ProbeWorld {
  const marks = readGatewayMarks();
  const ownerPauseUntil: Record<string, number> = {};
  try {
    const path = join(homedir(), ".vitriol", "officina", "gateway-health.json");
    const doc = JSON.parse(readFileSync(path, "utf8")) as {
      pauses?: Record<string, { until?: number }>;
    };
    for (const [owner, pause] of Object.entries(doc.pauses ?? {})) {
      ownerPauseUntil[owner] = Number(pause?.until ?? 0);
    }
  } catch {
    // no pauses
  }
  return {
    probes: marks.probes,
    invalid: marks.invalid,
    pausedOwners: marks.pausedOwners,
    coolingUntil: coolingMap(),
    ownerPauseUntil,
  };
}

async function probeLane(
  baseUrl: string,
  key: string,
  lane: string,
): Promise<{ ok: boolean; status: number; text: string; latencyMs: number }> {
  const started = Date.now();
  try {
    const res = await fetch(`${baseUrl}/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(key ? { Authorization: `Bearer ${key}` } : {}),
        "X-OmniRoute-No-Cache": "true",
      },
      body: JSON.stringify({
        model: lane,
        messages: [{ role: "user", content: "ping" }],
        max_tokens: 1,
        temperature: 0,
      }),
      signal: AbortSignal.timeout(PROBE_TIMEOUT_MS),
    });
    const text = await res.text();
    return { ok: res.ok, status: res.status, text, latencyMs: Date.now() - started };
  } catch (error) {
    return {
      ok: false,
      status: 0,
      text: error instanceof Error ? error.message : String(error),
      latencyMs: Date.now() - started,
    };
  }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_PROBE === "1") return;
  const pid = process.pid;
  let bucket: RateBucket = emptyBucket(Date.now());
  let timer: ReturnType<typeof setInterval> | undefined;
  let busy = false;
  let pauseUntil = 0;

  const tick = async (): Promise<void> => {
    if (busy) return;
    const now = Date.now();
    if (!acquireProbeLease(pid, LEASE_TTL_MS)) return; // another session probes
    busy = true;
    try {
      const lanes = loadLanes();
      if (lanes.length === 0) return;
      const w = world();
      if (now < pauseUntil) return;
      if (isExhausted(lanes, w, now)) {
        // Truly exhausted: keep a slow heartbeat (operator requirement).
        pauseUntil = now + HEARTBEAT_MS;
        emitHarnessEvent(harnessEvent("lc-probe", "exhausted", { detail: `${lanes.length} lane(s)` }));
        return;
      }
      const choice = selectProbeCandidate(lanes, w, now, VERIFIED_TTL_MS);
      if (!choice) return;
      const gate = rateAllow(bucket, now, PER_MIN, DAILY);
      bucket = gate.bucket;
      if (!gate.allow) return;

      const cfg = envConfig();
      const lane = choice.lane.id.replace(/^caduceus\//, "");
      const owner = choice.lane.owner;
      const res = await probeLane(cfg.baseUrl, cfg.key, lane);
      const prev = w.probes[lane];
      const failCount = res.ok ? 0 : Number(prev?.failCount ?? 0) + 1;

      const baseFact = (kind: ProbeFact["kind"], extra: Partial<ProbeFact> = {}): ProbeFact => ({
        kind,
        failCount,
        latencyMs: res.latencyMs,
        nextProbeAt: res.ok ? now + VERIFIED_TTL_MS : now + probeBackoffMs(failCount),
        ...extra,
      });

      if (res.ok) {
        markProbe(lane, baseFact("ok", { okAt: now }));
        recordSuccess("caduceus", lane);
        clearOwnerPause(owner);
      } else {
        const cooldown = parseCooldownFromError(res.text, now);
        const notFound = parseModelNotFound(res.text);
        const pause = parseOwnerPause(res.text);
        const limit = parseContextLimit(res.text);
        if (cooldown) {
          recordFailure("caduceus", lane, res.status || 429, {
            retryAfterMs: cooldown,
            errorClass: "model_cooldown",
          });
          markProbe(lane, baseFact("cooling", { failAt: now, until: now + cooldown }));
        } else if (notFound) {
          markGatewayInvalid({
            model: lane,
            reason: `probe 404; upstream: ${notFound.alternatives.join(", ").slice(0, 160)}`,
          });
          markOffered(owner, notFound.alternatives);
          markProbe(lane, baseFact("invalid", { failAt: now }));
        } else if (pause) {
          markOwnerPause(owner, pause, "probe 400 key pause");
          markProbe(lane, baseFact("paused", { failAt: now, until: now + pause.pauseMs }));
        } else if (limit) {
          markModelLimit(lane, limit);
          markProbe(lane, baseFact("no_fit", { failAt: now }));
        } else {
          markProbe(lane, baseFact("no_fit", { failAt: now }));
        }
      }

      emitHarnessEvent(
        harnessEvent("lc-probe", res.ok ? "ok" : "fail", {
          detail: `${lane} (${choice.why}) ${res.status || "err"} ${res.latencyMs}ms`,
        }),
      );
    } finally {
      busy = false;
    }
  };

  pi.on("session_start", () => {
    if (timer) return;
    timer = setInterval(() => void tick(), TICK_MS);
    void tick();
  });

  pi.on("session_shutdown", () => {
    if (timer) {
      clearInterval(timer);
      timer = undefined;
    }
    releaseProbeLease(pid);
  });
}
