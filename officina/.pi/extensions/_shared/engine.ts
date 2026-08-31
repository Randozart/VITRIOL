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
import { busySlots, counterDelta, parseMetrics, parseSlots, type SlotInfo } from "../vitriol-decode/decode.ts";

const DEFAULT_ENDPOINT = "http://127.0.0.1:8279";
const DEFAULT_POLL_MS = 700;

export interface EngineSnapshot {
  up: boolean;
  /** token delta since previous poll + derived t/s */
  delta: { tps: number; tokens: number };
  /** context ingestion (prefill) delta + derived tokens/s */
  ingest: { tps: number; tokens: number };
  /** decode tokens since engine boot */
  total: number;
  slots: SlotInfo[];
  busy: number;
}

type Listener = () => void;

let timer: ReturnType<typeof setInterval> | undefined;
let listeners = new Set<Listener>();
let snap: EngineSnapshot = {
  up: false,
  delta: { tps: 0, tokens: 0 },
  ingest: { tps: 0, tokens: 0 },
  total: 0,
  slots: [],
  busy: 0,
};

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
  const now = Date.now();
  const metricsText = await fetchText(`${base}/metrics`, pollMs);
  if (metricsText === null) {
    before = null;
    if (snap.up || !polledOnce) {
      polledOnce = true;
      snap = { ...snap, up: false, busy: 0, ingest: { tps: 0, tokens: 0 } };
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
  snap = { up: true, delta, ingest, total: after.decodeTokens, slots, busy };
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
