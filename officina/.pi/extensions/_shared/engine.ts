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
import { busySlots, counterDelta, gpuFireLoad, parseMetrics, parseSlots, type SlotInfo } from "../vitriol-decode/decode.ts";

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

async function fetchText(url: string, timeoutMs: number): Promise<string | null> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), timeoutMs);
    const res = await fetch(url, { signal: ctrl.signal });
    clearTimeout(t);
    if (!res.ok) return null;
    return await res.text();
  } catch {
    return null;
  }
}

let before: ReturnType<typeof parseMetrics> = null;
let lastPoll = 0;
let polledOnce = false;
let base = "";
let pollMs = DEFAULT_POLL_MS;

async function poll(): Promise<void> {
  pollNvidiaSmi();
  const now = Date.now();
  const metricsText = await fetchText(`${base}/metrics`, pollMs);
  if (metricsText === null) {
    before = null;
    if (snap.up || !polledOnce) {
      polledOnce = true;
      snap = { ...snap, up: false, busy: 0, ingest: { tps: 0, tokens: 0 }, cumulativeIngest: 0 };
      notify();
    }
    return;
  }
  const after = parseMetrics(metricsText);
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
  const slotsText = await fetchText(`${base}/slots`, pollMs);
  const slots = slotsText ? parseSlots(slotsText) : snap.slots;
  const busy = busySlots(slots, delta.tokens);
  // cumulativeIngest = total prompt tokens processed since engine boot
  snap = { up: true, delta, ingest, cumulativeIngest: after.promptTokens, total: after.decodeTokens, slots, busy, gpuLoad: gpuLoadLatest };
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
