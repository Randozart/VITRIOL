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
import { busySlots, counterDelta, gpuFireLoad, parseLoadedModel, parseMetrics, parseModelPath, parseSlots, type SlotInfo } from "../vitriol-decode/decode.ts";

const DEFAULT_ENDPOINT = "http://127.0.0.1:8279";
const DEFAULT_POLL_MS = 700;

export interface EngineSnapshot {
  up: boolean;
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
  /** True when the engine answers TCP but queue-backed endpoints stall
   * (generation in flight) — alive-but-busy, NOT down. */
  stalled: boolean;
  /** decode tokens since engine boot */
  total: number;
  slots: SlotInfo[];
  busy: number;
  /** GPU fire load in [0,1] — power.draw/power.limit, max across GPUs
   *  (composer flames, owner request 2026-09-02); null when nvidia-smi
   *  is absent or has not answered yet. */
  gpuLoad: number | null;
}

type Listener = () => void;

let timer: ReturnType<typeof setInterval> | undefined;
let listeners = new Set<Listener>();
let snap: EngineSnapshot = {
  up: false,
  delta: { tps: 0, tokens: 0 },
  ingest: { tps: 0, tokens: 0 },
  cumulativeIngest: 0,
  ejected: 0,
  loaded_model: "",
  loaded_path: "",
  stalled: false,
  total: 0,
  slots: [],
  busy: 0,
  gpuLoad: null,
};

// ── GPU fire load (composer flames, owner request 2026-09-02) ────────────
// One nvidia-smi spawn per poll tick rides the existing 700ms loop. The
// exec is fire-and-forget: the next poll() reads the freshest completed
// answer, so a slow nvidia-smi never delays telemetry. ENOENT latches off
// (no NVIDIA driver → stop spawning forever). Never throws (observability
// contract).
let nvidiaMissing = false;
let gpuLoadLatest: number | null = null;

function pollNvidiaSmi(): void {
  if (nvidiaMissing) return;
  execFile(
    "nvidia-smi",
    ["--query-gpu=power.draw,power.limit,utilization.gpu", "--format=csv,noheader,nounits"],
    { timeout: 1500 },
    (err, stdout) => {
      if (err) {
        if ((err as NodeJS.ErrnoException).code === "ENOENT") nvidiaMissing = true;
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
      if (saw) gpuLoadLatest = load;
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

let before: ReturnType<typeof parseMetrics> = null;
let lastPoll = 0;
let polledOnce = false;
let base = "";
let pollMs = DEFAULT_POLL_MS;
// Endpoint fetch timeout — rides short stalls; poll CADENCE stays pollMs.
const HTTP_TIMEOUT_MS = 1500;

async function poll(): Promise<void> {
  pollNvidiaSmi();
  const now = Date.now();
  const metrics = await fetchText(`${base}/metrics`, HTTP_TIMEOUT_MS);
  if (metrics.kind !== "ok") {
    before = null;
    polledOnce = true;
    if (metrics.kind === "down") {
      snap = { ...snap, up: false, stalled: false, busy: 0, ingest: { tps: 0, tokens: 0 }, cumulativeIngest: 0, ejected: 0, loaded_model: "", loaded_path: "" };
    } else {
      // alive-but-busy: queue-backed endpoint didn't answer in time
      snap = { ...snap, up: true, stalled: true };
    }
    notify();
    return;
  }
  const after = parseMetrics(metrics.text);
  if (!after) return;
  const seconds = lastPoll ? (now - lastPoll) / 1000 : 0;
  const delta = counterDelta(before, after, seconds);
  // ingestion (prefill) rate from the prompt-tokens counter, same math
  const ingestTokens = before ? Math.max(0, after.promptTokens - before.promptTokens) : 0;
  const ingest = {
    tps: seconds > 0 ? ingestTokens / seconds : 0,
    tokens: ingestTokens,
  };
  before = after;
  lastPoll = now;
  const slotsOut = await fetchText(`${base}/slots`, HTTP_TIMEOUT_MS);
  const slots = slotsOut.kind === "ok" ? parseSlots(slotsOut.text) : snap.slots;
  const busy = busySlots(slots, delta.tokens);
  // The ENGINE's loaded model (owner request 2026-09-03): /v1/models
  // reports the alias of whatever the server actually loaded — distinct
  // from pi's selected-model label.
  const modelsOut = await fetchText(`${base}/v1/models`, HTTP_TIMEOUT_MS);
  const loaded_model = modelsOut.kind === "ok" ? parseLoadedModel(modelsOut.text) : snap.loaded_model;
  const propsOut = await fetchText(`${base}/props`, HTTP_TIMEOUT_MS);
  const loaded_path = propsOut.kind === "ok" ? parseModelPath(propsOut.text) : snap.loaded_path;
  // Secondary endpoints queue-wait by design (/slots) — their stall is the
  // live busy signal once /metrics itself is non-blocking (engine 2026-09-04).
  const stalled = slotsOut.kind === "stalled" || modelsOut.kind === "stalled" || propsOut.kind === "stalled";
  // cumulativeIngest = total prompt tokens processed since engine boot
  snap = { up: true, stalled, delta, ingest, cumulativeIngest: after.promptTokens, total: after.decodeTokens, slots, busy, gpuLoad: gpuLoadLatest, ejected: after.ejected ?? 0, loaded_model, loaded_path };
  polledOnce = true;
  notify();
}

function notify(): void {
  for (const l of [...listeners]) {
    try {
      l();
    } catch {
      // a listener must never break the poll loop
    }
  }
}

/** Idempotent: starts the shared poll loop (no-op if already running). */
export function startEnginePolling(opts?: { base?: string; pollMs?: number }): void {
  base = (opts?.base || process.env.VITRIOL_BASE_URL || DEFAULT_ENDPOINT).replace(/\/$/, "");
  pollMs = Math.max(200, Number(opts?.pollMs ?? process.env.VITRIOL_DECODE_POLL_MS) || DEFAULT_POLL_MS);
  if (timer) return;
  timer = setInterval(() => void poll(), pollMs);
  void poll();
}

export function getEngineSnapshot(): EngineSnapshot {
  return snap;
}

export function onEngineUpdate(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export { busySlots };
