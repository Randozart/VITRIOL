// Shared engine telemetry poller (2026-08-31, sidebar content plan step 1:
// .opencode/plans/officina-sidebar-content-2026-08-31.md). Lifted from
// vitriol-decode/index.ts so the docked sidebar and the decode widget share
// ONE poll loop instead of two pollers hitting /metrics and /slots.
//
// Pure parsers stay in vitriol-decode/decode.ts (unit-tested there); this
// module owns the polling loop and the shared snapshot. Never throws, never
// blocks the agent loop (observability contract).
//
// Provenance: original work, this repo; parsers from vitriol-decode/decode.ts
// (own; this repo, Apache-2.0 OR MIT).
import { execFile } from "node:child_process";
import { busySlots, counterDelta, gpuFireLoad, parseLoadedModel, parseMetrics, parseModelPath, parseRpcServers, parseSlots, type SlotInfo } from "../vitriol-decode/decode.ts";

const DEFAULT_ENDPOINT = "http://127.0.0.1:8279";
const DEFAULT_POLL_MS = 700;

export interface EngineSnapshot {
  up: boolean;
  /** True once the first poll attempt has completed (success OR failure).
   *  Failover routing must not treat the pre-poll window as "engine down". */
  polled: boolean;
  /** token delta since previous poll + derived t/s */
  delta: { tps: number; tokens: number };
  /** context ingestion (prefill) delta + derived tokens/s */
  ingest: { tps: number; tokens: number };
  /** cumulative prompt tokens since engine boot (for ingestion gauge) */
  cumulativeIngest: number;
  /** VITRIOL sparse-KV: KV cells ejected since engine boot (0 if unsupported). */
  ejected: number;
  /** The model id the ENGINE actually loaded (alias), "" when unknown. */
  loaded_model: string;
  /** The loaded model's file path (from /props), "" when unknown. */
  loaded_path: string;
  /** RPC (row-split) server endpoints the engine loaded (from /props),
   *  e.g. ["100.92.76.67:50052"]; [] when local-only. */
  rpc: string[];
  /** True when the engine answers TCP but queue-backed endpoints stall
   * (generation in flight) — alive-but-busy, NOT down. */
  stalled: boolean;
  /** decode tokens since engine boot */
  total: number;
  /** Slot table. Persists across /slots stalls (last-known), so once a
   *  poll has succeeded `slots.length` is trustworthy for arbitration. */
  slots: SlotInfo[];
  /** True once ONE /slots poll has succeeded — peer arbitration uses the
   *  last-known slot table through queue-backed stalls instead of
   *  withholding the peer arm exactly under contention (2026-11-11). */
  slotsSeen: boolean;
  busy: number;
  /** Recent queue wait from the last poll interval (ms); 0 when no new
   *  requests. Overflow routing uses this as the local-contention signal. */
  queueWaitMs: number;
  /** Engine gauges: in-flight / deferred requests at the last poll. */
  processing: number;
  deferred: number;
  /** GPU fire load in [0,1] — power.draw/power.limit, max across GPUs
   *  (composer flames, owner request 2026-09-02); null when nvidia-smi
   *  is absent or has not answered yet. */
  gpuLoad: number | null;
}

type Listener = () => void;

// globalThis-backed state (2026-09-06): pi loads each -e extension via a
// SEPARATE jiti instance (moduleCache: false) — module-scope state does not
// cross extension boundaries. engine.ts is imported by session-panel,
// vitriol-decode AND background-lane; without sharing, each instance ran its
// own poller (3x HTTP load on /metrics + /slots, tripled stall noise) and
// readers with no poller saw a frozen snapshot. One shared object = one
// poller (the state.timer guard now works across instances), one listener set,
// one snapshot.
interface EngineState {
  timer: ReturnType<typeof setInterval> | undefined;
  listeners: Set<Listener>;
  snap: EngineSnapshot;
  nvidiaMissing: boolean;
  gpuLoadLatest: number | null;
  before: ReturnType<typeof parseMetrics>;
  lastPoll: number;
  polledOnce: boolean;
  /** In-flight guard: setInterval must never overlap two polls. Overlap
   *  let one poll overwrite `before`/`lastPoll` while another was still
   *  reading them, which poisoned the delta/tok-s math under endpoint
   *  stalls (2026-11-10). */
  polling: boolean;
  base: string;
  pollMs: number;
}
const state: EngineState =
  (globalThis as any).__officinaEngineState ??
  ((globalThis as any).__officinaEngineState = {
    timer: undefined,
    listeners: new Set<Listener>(),
    snap: {
      up: false,
      polled: false,
      delta: { tps: 0, tokens: 0 },
      ingest: { tps: 0, tokens: 0 },
      cumulativeIngest: 0,
      ejected: 0,
      loaded_model: "",
      loaded_path: "",
      rpc: [],
      stalled: false,
      total: 0,
      slots: [],
      slotsSeen: false,
      busy: 0,
      queueWaitMs: 0,
      processing: 0,
      deferred: 0,
      gpuLoad: null,
    },
    nvidiaMissing: false,
    gpuLoadLatest: null,
    before: null,
    lastPoll: 0,
    polledOnce: false,
    polling: false,
    base: "",
    pollMs: DEFAULT_POLL_MS,
  });

// ── GPU fire load (composer flames, owner request 2026-09-02) ────────────
// One nvidia-smi spawn per poll tick rides the existing 700ms loop. The
// exec is fire-and-forget: the next poll() reads the freshest completed
// answer, so a slow nvidia-smi never delays telemetry. ENOENT latches off
// (no NVIDIA driver → stop spawning forever). Never throws (observability
// contract).


function pollNvidiaSmi(): void {
  if (state.nvidiaMissing) return;
  execFile(
    "nvidia-smi",
    ["--query-gpu=power.draw,power.limit,utilization.gpu", "--format=csv,noheader,nounits"],
    { timeout: 1500 },
    (err, stdout) => {
      if (err) {
        if ((err as NodeJS.ErrnoException).code === "ENOENT") state.nvidiaMissing = true;
        return;
      }
      let load = 0;
      let saw = false;
      for (const line of stdout.split("\n")) {
        const cols = line.split(",").map((s) => Number(s.trim()));
        const [p, lim, util] = cols;
        if (!Number.isFinite(p) || !Number.isFinite(lim) || lim <= 0) continue;
        saw = true;
        load = Math.max(load, gpuFireLoad(p, lim, Number.isFinite(util) ? util : -1));
      }
      if (saw) state.gpuLoadLatest = load;
    },
  );
}

export type FetchOutcome = { kind: "ok"; text: string } | { kind: "stalled" } | { kind: "down" };

/**
 * Map a fetch failure to stalled-vs-down. Abort (our timeout) after the
 * TCP connection succeeded = the engine is ALIVE but the endpoint
 * stalled (queue-backed /metrics and /slots block during generation).
 * Refused/reset/unreachable = nothing usable is listening = down.
 * (2026-09-04: /metrics queue-waited behind active generation, every
 * scrape timed out, and the poll read the timeouts as "engine down".)
 */
export function classifyFetchError(err: unknown): "stalled" | "down" {
  const code = (err as { cause?: { code?: string } })?.cause?.code ?? "";
  if (code === "ECONNREFUSED" || code === "ECONNRESET" || code === "EHOSTUNREACH" || code === "ENETUNREACH") {
    return "down";
  }
  if (err instanceof Error && err.name === "AbortError") return "stalled";
  return "down";
}

async function fetchText(url: string, timeoutMs: number): Promise<FetchOutcome> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), timeoutMs);
    const res = await fetch(url, { signal: ctrl.signal });
    clearTimeout(t);
    if (!res.ok) return { kind: "down" };
    return { kind: "ok", text: await res.text() };
  } catch (err) {
    return { kind: classifyFetchError(err) };
  }
}


// Endpoint fetch timeout — rides short stalls; poll CADENCE stays state.pollMs.
const HTTP_TIMEOUT_MS = 1500;

async function poll(): Promise<void> {
  // One poll at a time (2026-11-10): the interval is 700 ms but a stalled
  // /metrics + /slots + /v1/models + /props chain can take seconds. Without
  // this guard those ticks overlapped and each one rewrote `before` /
  // `lastPoll` while another was mid-read, so the tok/s delta was divided
  // by the wrong interval (or double-counted tokens). Serialize the loop.
  if (state.polling) return;
  state.polling = true;
  try {
    pollNvidiaSmi();
    const metrics = await fetchText(`${state.base}/metrics`, HTTP_TIMEOUT_MS);
    // Sample time is when the counters were READ, not when the request
    // started: a queue-backed /metrics can answer seconds later, and using
    // the request-start clock understated/overstated the rate.
    const sampledAt = Date.now();
    if (metrics.kind !== "ok") {
      state.before = null;
      state.polledOnce = true;
      if (metrics.kind === "down") {
        state.snap = { ...state.snap, up: false, polled: true, stalled: false, busy: 0, queueWaitMs: 0, processing: 0, deferred: 0, ingest: { tps: 0, tokens: 0 }, cumulativeIngest: 0, ejected: 0, loaded_model: "", loaded_path: "", rpc: [] };
      } else {
        // alive-but-busy: queue-backed endpoint didn't answer in time
        state.snap = { ...state.snap, up: true, polled: true, stalled: true };
      }
      notify();
      return;
    }
    const after = parseMetrics(metrics.text);
    if (!after) return;
    const seconds = state.lastPoll ? (sampledAt - state.lastPoll) / 1000 : 0;
    const delta = counterDelta(state.before, after, seconds);
    // ingestion (prefill) rate from the prompt-tokens counter, same math
    const ingestTokens = state.before ? Math.max(0, after.promptTokens - state.before.promptTokens) : 0;
    const ingest = {
      tps: seconds > 0 ? ingestTokens / seconds : 0,
      tokens: ingestTokens,
    };
    let queueWaitMs = 0;
    if (
      state.before &&
      after.queueCount !== undefined &&
      state.before.queueCount !== undefined
    ) {
      const dc = after.queueCount - state.before.queueCount;
      const ds = (after.queueSum ?? 0) - (state.before.queueSum ?? 0);
      if (dc > 0) queueWaitMs = Math.max(0, (ds / dc) * 1000);
    }
    state.before = after;
    state.lastPoll = sampledAt;
    const slotsOut = await fetchText(`${state.base}/slots`, HTTP_TIMEOUT_MS);
    const slots = slotsOut.kind === "ok" ? parseSlots(slotsOut.text) : state.snap.slots;
    const slotsSeen = slotsOut.kind === "ok" || state.snap.slotsSeen;
    const busy = busySlots(slots, delta.tokens);
    // The ENGINE's loaded model (owner request 2026-09-03): /v1/models
    // reports the alias of whatever the server actually loaded — distinct
    // from pi's selected-model label.
    const modelsOut = await fetchText(`${state.base}/v1/models`, HTTP_TIMEOUT_MS);
    const loaded_model = modelsOut.kind === "ok" ? parseLoadedModel(modelsOut.text) : state.snap.loaded_model;
    const propsOut = await fetchText(`${state.base}/props`, HTTP_TIMEOUT_MS);
    const loaded_path = propsOut.kind === "ok" ? parseModelPath(propsOut.text) : state.snap.loaded_path;
    const rpc = propsOut.kind === "ok" ? parseRpcServers(propsOut.text) : state.snap.rpc;
    // Secondary endpoints queue-wait by design (/slots) — their stall is the
    // live busy signal once /metrics itself is non-blocking (engine 2026-09-04).
    const stalled = slotsOut.kind === "stalled" || modelsOut.kind === "stalled" || propsOut.kind === "stalled";
    // cumulativeIngest = total prompt tokens processed since engine boot
    state.snap = { up: true, polled: true, stalled, delta, ingest, cumulativeIngest: after.promptTokens, total: after.predictedTokens, slots, slotsSeen, busy, queueWaitMs, processing: after.processing ?? 0, deferred: after.deferred ?? 0, gpuLoad: state.gpuLoadLatest, ejected: after.ejected ?? 0, loaded_model, loaded_path, rpc };
    state.polledOnce = true;
    notify();
  } finally {
    state.polling = false;
  }
}

function notify(): void {
  for (const l of [...state.listeners]) {
    try {
      l();
    } catch {
      // a listener must never break the poll loop
    }
  }
}

/** Idempotent: starts the shared poll loop (no-op if already running). */
export function startEnginePolling(opts?: { base?: string; pollMs?: number }): void {
  state.base = (opts?.base || process.env.VITRIOL_BASE_URL || DEFAULT_ENDPOINT).replace(/\/$/, "");
  state.pollMs = Math.max(200, Number(opts?.pollMs ?? process.env.VITRIOL_DECODE_POLL_MS) || DEFAULT_POLL_MS);
  if (state.timer) return;
  state.timer = setInterval(() => void poll(), state.pollMs);
  // unref (2026-09-27): telemetry must never pin the process. TUI callers
  // keep node alive anyway; print/json mode must be allowed to exit even if
  // a stray consumer started the loop.
  state.timer.unref?.();
  void poll();
}

export function getEngineSnapshot(): EngineSnapshot {
  return state.snap;
}

export function onEngineUpdate(fn: Listener): () => void {
  state.listeners.add(fn);
  return () => state.listeners.delete(fn);
}

/** Test hook (jiti-isolation guard): true when THIS module instance sees a
 *  live poller. After the globalThis fix all instances share the timer, so
 *  instance B reports active after only instance A started polling. */
export function enginePollerActive(): boolean {
  return state.timer !== undefined;
}

/** Test hook / shutdown: stop the shared poll loop. */
export function stopEnginePolling(): void {
  if (state.timer) {
    clearInterval(state.timer);
    state.timer = undefined;
  }
}

export { busySlots };
