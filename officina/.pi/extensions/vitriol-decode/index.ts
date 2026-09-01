import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { fmtRate, fmtTokens } from "./decode.ts";
import { RAMPS, renderGauge } from "./braille.ts";
import { getEngineSnapshot, onEngineUpdate, startEnginePolling } from "../_shared/engine.ts";

// vitriol-decode — live engine telemetry below the editor (2026-08-31).
//
// First-Party Mandate standing requirement (AGENTS.md 2026-08-31, owner):
// the cockpit shows live decode progress — streamed tokens, t/s, slot
// state — so the VITRIOL TUI is never needed to see how far along the
// local agent is. This widget polls the engine's /metrics (cumulative
// counters, fork names verified 2026-08-29) and /slots (slot busy state),
// deltas them locally, and renders a Crush-grade status bar below the
// editor. Generic agents show a spinner; we show ENGINE TRUTH.
//
// 2026-09-01: restored to belowEditor after the sidebar experiment —
// live gauges belong next to the composer (owner decision). The sidebar
// keeps the precise eng/ing numbers (session-panel); gauges here,
// numbers there.
//
// Kill switch: VITRIOL_DECODE_WIDGET=0 (Rule 15). Poll interval:
// VITRIOL_DECODE_POLL_MS (default 700). Engine endpoint: VITRIOL_BASE_URL
// (default http://127.0.0.1:8279, same env the bridge and tris use).
//
// Never throws, never blocks the agent loop (observability contract,
// same discipline as _shared/events.ts). Widget renders "engine down"
// honestly instead of going silent.

export default function (pi: ExtensionAPI) {
  if (process.env.VITRIOL_DECODE_WIDGET === "0") return; // Rule 15

  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ui is ctx-bound; typed loosely here, set in session_start
  let ui: any = null;
  let sessionDir = "";

  const render = () => {
    if (!ui) return; // no session context yet; engine updates render once bound
    const last = getEngineSnapshot();
    if (!last.up) {
      ui.setWidget("vitriol-decode", [
        `◈ VITRIOL  engine down (tris up to start)`,
      ], { placement: "belowEditor" });
      return;
    }
    // Vitriolum visual language (matches the engine TUI): slot capacity on
    // the white→yellow→orange→red ramp, decode activity on the
    // teal→green→cyan ramp, idle rendered on the mercury ramp.
    // Busy truth is TWO-source (2026-08-31 bugfix): slot is_processing
    // flags AND token movement — see busySlots() in decode.ts.
    // renderGauge delegates to the Rust addon when built (same output).
    const busy = last.busy;
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

  pi.on("session_start", (_event, ctx) => {
    if (!ctx.hasUI) return; // print/JSON mode: nothing to decorate
    ui = ctx.ui;
    sessionDir = ctx.cwd;
    try {
      // terminal tab / window title: workshop + which folder
      const title = `officina · ${ctx.cwd.replace(/^\/home\/[^/]+/, "~")}`;
      ctx.ui.setTitle?.(title);
    } catch {
      // title is decoration, never load-bearing
    }
    startEnginePolling();
    onEngineUpdate(render);
    render();
  });
}
