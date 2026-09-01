import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { getEngineSnapshot, startEnginePolling } from "../_shared/engine.ts";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  PatchSink,
  buildReviewPrompt,
  laneConfig,
  renderCard,
  shouldLaunch,
  type LaneConfig,
} from "./lane.ts";

// background-lane — idle-gated diff reviewer on the engine's second slot
// (plan: .opencode/plans/dual-slot-background-lane-2026-08-31.md, v1 idea 1+2).
//
// Contract: bounded input (the turn's accumulated edit patches) → one compact
// findings card → injected ONCE into the main context. Launch gate: the
// engine must be fully idle (all slots idle, zero decode) for idleMs — the
// lane converts between-turn idle cycles into work and never competes with
// foreground decode by design. Requires the engine to run with `-np 2` (or
// more); with fewer slots the gate still holds but jobs and the foreground
// would contend — see OFFICINA_BG_SLOT.
//
// Kill switch: OFFICINA_NO_BACKGROUND=1 (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg: LaneConfig = laneConfig();
  if (!cfg.enabled) return; // Rule 15

  const sink = new PatchSink();
  let idleSince: number | null = null;
  let launched = false; // one job in flight
  let sessionStem = "session";
  let cardsDir = "";
  let cards = 0;
  const pending: string[] = [];

  // --- input collection: patches from edit/write tool results -------------

  pi.on("tool_result", (event) => {
    const name = String(event.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if ((event as { isError?: boolean }).isError) return;
    const p = String((event.input as Record<string, unknown> | undefined)?.path
      ?? (event.input as Record<string, unknown> | undefined)?.file_path ?? "");
    if (!p) return;
    const details = (event as { details?: { patch?: string; diff?: string } }).details;
    sink.add(p, details?.patch ?? details?.diff);
  });

  // --- gate + launch loop (rides the shared engine poller) ----------------

  const tryLaunch = async () => {
    if (launched) return;
    const snap = getEngineSnapshot();
    const now = Date.now();
    if (!snap.up) return;
    if (cfg.gate === "idle") {
      if (snap.busy > 0 || snap.delta.tps > 0) {
        idleSince = null; // foreground is active — reset the idle window
        return;
      }
      if (idleSince === null) idleSince = now;
    }
    if (!shouldLaunch(snap, sink.size, idleSince, now, cfg)) return;
    const patch = sink.drain(cfg.minPatchChars, cfg.maxPatchChars);
    if (!patch) return;
    launched = true;
    idleSince = null;
    emitHarnessEvent(harnessEvent("lc-bg", "launch", { detail: `${patch.length}ch` }));
    try {
      const body: Record<string, unknown> = {
        prompt: buildReviewPrompt(patch, cfg),
        n_predict: cfg.maxTokens,
        temperature: 0.2,
        cache_prompt: true, // REVIEWER_PREFIX is byte-stable → KV reuse
        stream: false,
      };
      if (cfg.slotId !== undefined) body.slot_id = cfg.slotId;
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), cfg.timeoutMs);
      const res = await fetch(`${cfg.base}/completion`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: ctrl.signal,
      });
      clearTimeout(timer);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as { content?: string; text?: string };
      const response = data.content ?? data.text ?? "";
      const card = renderCard("this turn's edits", response, cfg);
      if (card) {
        cards++;
        try {
          mkdirSync(cardsDir, { recursive: true });
          writeFileSync(join(cardsDir, `review-${String(cards).padStart(3, "0")}.md`), card + "\n");
        } catch {
          /* card file is a convenience; injection below is the delivery */
        }
        pending.push(card);
        emitHarnessEvent(harnessEvent("lc-bg", "findings", { detail: card.slice(0, 200) }));
      }
    } catch {
      emitHarnessEvent(harnessEvent("lc-bg", "error", { detail: "review request failed" }));
    } finally {
      launched = false;
    }
  };

  // --- delivery: inject each card exactly once as a tail message ----------

  pi.on("context", async (event) => {
    if (pending.length === 0) return undefined;
    const block = pending.join("\n\n");
    pending.length = 0;
    const tail = {
      role: "custom" as const,
      customType: "lc-bg",
      content: "\n\n" + block,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });

  pi.on("session_start", (_event, ctx) => {
    try {
      sessionStem = (ctx.sessionManager.getSessionId() ?? "session").replace(/[^a-zA-Z0-9._-]/g, "_").slice(0, 40);
    } catch {
      sessionStem = "session";
    }
    cardsDir = join(ctx.cwd, ".pi", "background", sessionStem);
    if (ctx.hasUI) startEnginePolling();
    // one launch attempt per poll tick — the poller is the heartbeat
    setInterval(() => void tryLaunch(), Math.max(1000, Math.floor(cfg.idleMs / 2)));
  });
}
