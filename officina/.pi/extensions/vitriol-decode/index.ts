import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  counterDelta,
  fmtRate,
  fmtTokens,
  parseMetrics,
  type MetricsCounters,
} from "./decode.ts";
import { RAMPS, renderGauge } from "./braille.ts";
import { busySlots, parseSlots, type SlotInfo } from "./decode.ts";

// vitriol-decode — live engine telemetry in the editor (2026-08-31).
//
// First-Party Mandate standing requirement (AGENTS.md 2026-08-31, owner):
// the cockpit shows live decode progress — streamed tokens, t/s, slot
// state — so the VITRIOL TUI is never needed to see how far along the
// local agent is. This widget polls the engine's /metrics (cumulative
// counters, fork names verified 2026-08-29) and /slots (slot busy state),
// deltas them locally, and renders a Crush-grade status bar below the
// editor. Generic agents show a spinner; we show ENGINE TRUTH.
//
// Kill switch: VITRIOL_DECODE_WIDGET=0 (Rule 15). Poll interval:
// VITRIOL_DECODE_POLL_MS (default 700). Engine endpoint: VITRIOL_BASE_URL
// (default http://127.0.0.1:8279, same env the bridge and tris use).
//
// Never throws, never blocks the agent loop (observability contract,
// same discipline as _shared/events.ts). Widget renders "engine down"
// honestly instead of going silent.

const DEFAULT_ENDPOINT = "http://127.0.0.1:8279";
const DEFAULT_POLL_MS = 700;

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

export default function (pi: ExtensionAPI) {
  if (process.env.VITRIOL_DECODE_WIDGET === "0") return; // Rule 15

  const base = (process.env.VITRIOL_BASE_URL || DEFAULT_ENDPOINT).replace(/\/$/, "");
  const pollMs = Math.max(200, Number(process.env.VITRIOL_DECODE_POLL_MS) || DEFAULT_POLL_MS);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ui is ctx-bound; typed loosely here, set in session_start
  let ui: any = null;
  let before: MetricsCounters | null = null;
  let lastPoll = 0;
  let polledOnce = false;
  let sessionDir = "";
  let last: { delta: { tps: number; tokens: number }; total: number; slots: SlotInfo[]; up: boolean } = {
    delta: { tps: 0, tokens: 0 },
    total: 0,
    slots: [],
    up: false,
  };

  const render = () => {
    if (!ui) return; // no session context yet; poll() renders once bound
    if (!last.up) {
      ui.setWidget("vitriol-decode", [
        `◈ VITRIOL  engine down @ ${base}  (tris up to start)`,
      ], { placement: "belowEditor" });
      return;
    }
    // Vitriolum visual language (matches the engine TUI): slot capacity on
    // the white→yellow→orange→red ramp, decode activity on the
    // teal→green→cyan ramp, idle rendered on the mercury ramp.
    // Busy truth is TWO-source (2026-08-31 bugfix): slot is_processing
    // flags AND token movement — see busySlots() in decode.ts.
    const slotBusy = busySlots(last.slots, last.delta.tokens);
    const busy = slotBusy;
    const total = Math.max(last.slots.length, busy, 1);
    const decoding = last.delta.tps > 0 || busy > 0;
    const slotGauge = renderGauge(RAMPS.capacity, busy / total, 10);
    const ratio = Math.min(1, last.delta.tps / 25); // ramp saturates at 25 tok/s
    const tpsGauge = renderGauge(decoding ? RAMPS.activity : RAMPS.mercury, decoding ? Math.max(0.08, ratio) : 0.08, 10);
    const line1 =
      `◈ VITRIOL  ${slotGauge} slots ${busy}/${total}` +
      `   ${tpsGauge} ${decoding ? `${fmtRate(last.delta.tps)} tok/s` : "idle"}` +
      `   ·  ${fmtTokens(last.total)} decoded this boot`;
    const line2 = sessionDir ? `◈ ${sessionDir}` : undefined;
    ui.setWidget("vitriol-decode", line2 ? [line1, line2] : [line1], { placement: "belowEditor" });
  };

  const poll = async () => {
    const now = Date.now();
    const metricsText = await fetchText(`${base}/metrics`, pollMs);
    if (metricsText === null) {
      if (last.up || !polledOnce) {
        polledOnce = true;
        last = { ...last, up: false };
        render();
      }
      before = null;
      return;
    }
    const after = parseMetrics(metricsText);
    if (!after) return;
    const seconds = lastPoll ? (now - lastPoll) / 1000 : 0;
    const delta = counterDelta(before, after, seconds);
    before = after;
    lastPoll = now;
    const slotsText = await fetchText(`${base}/slots`, pollMs);
    const slots = slotsText ? parseSlots(slotsText) : last.slots;
    last = { delta, total: after.decodeTokens, slots, up: true };
    polledOnce = true;
    render();
  };

  let timer: ReturnType<typeof setInterval> | undefined;
  pi.on("session_start", (_event, ctx) => {
    if (timer || !ctx.hasUI) return; // print/JSON mode: nothing to decorate
    ui = ctx.ui;
    sessionDir = ctx.cwd;
    try {
      // terminal tab / window title: workshop + which folder
      const title = `officina · ${ctx.cwd.replace(/^\/home\/[^/]+/, "~")}`;
      ctx.ui.setTitle?.(title);
    } catch {
      // title is decoration, never load-bearing
    }
    void poll();
    timer = setInterval(() => void poll(), pollMs);
    const t = timer;
    process.on("exit", () => clearInterval(t));
  });
}
