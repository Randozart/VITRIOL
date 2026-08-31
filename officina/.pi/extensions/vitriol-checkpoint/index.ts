import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { mkdirSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  ckptConfig,
  decodeMarker,
  encodeMarker,
  markerPath,
  parseSlotOpResponse,
  restoreIsCompatible,
  safeFilename,
  slotActionUrl,
  turnFilename,
  type CkptConfig,
  type SlotOpResult,
} from "./client.ts";

// vitriol-checkpoint — scaffold trigger for engine slot save/restore (§2.2,
// step 4). Endpoints exist in this fork (POST /slots/<id>?action=save|restore,
// requires --slot-save-path — set in the profiles). Autosaves every
// TRIS_CKPT_EVERY_TURNS turns (default 10, 0 = manual only), leaves a marker,
// and a new session RESTORES from the marker instead of re-prefilling turn 0.
// Filenames pair with the snapshot ext (turn-N) → rewind code + KV together.
// Kill switch: TRIS_NO_VITRIOL_CKPT=1. Layer rule: HTTP API only.

export default function (pi: ExtensionAPI) {
  const cfg = ckptConfig();
  if (!cfg.enabled) return;

  let sessionStem = "session";
  let lastSavedTurn = -1;
  let modelId = process.env.TRIS_CKPT_MODEL ?? "";

  /** One engine call; never throws — the checkpoint layer must not break a run. */
  async function slotOp(action: "save" | "restore", filename: string): Promise<SlotOpResult> {
    try {
      const res = await fetch(slotActionUrl(cfg, action), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ filename }),
        signal: AbortSignal.timeout(120_000), // GB-scale state writes on slow disks
      });
      let body: unknown = null;
      try {
        body = await res.json();
      } catch {
        body = null;
      }
      return parseSlotOpResponse(res.status, body);
    } catch (e) {
      return { ok: false, status: 0, error: (e as Error).message };
    }
  }

  function writeMarker(filename: string, turn: number): void {
    try {
      const p = markerPath(cfg.sessionDir, sessionStem);
      mkdirSync(dirname(p), { recursive: true });
      writeFileSync(p, encodeMarker({
        endpoint: cfg.endpoint, slot: cfg.slot, filename, turn,
        model: modelId, at: Date.now(),
      }));
    } catch {
      // marker trouble disables crash-restore only; saves still reported
    }
  }

  function clearMarker(): void {
    try {
      unlinkSync(markerPath(cfg.sessionDir, sessionStem));
    } catch {
      // no marker — fine
    }
  }

  pi.on("session_start", async (_event, ctx) => {
    const sm = (ctx as { sessionManager?: { getSessionFile?: () => string | null } }).sessionManager;
    sessionStem = (sm?.getSessionFile?.() ?? "").split("/").pop()?.replace(/\.jsonl$/, "") || "session";
    const mdl = (ctx as { model?: { id?: string } }).model;
    if (mdl?.id) modelId = mdl.id;

    // Crash recovery: a marker means the previous session ended after its last
    // save; restore so the model resumes warm instead of re-paying prefill.
    let raw = "";
    try {
      raw = readFileSync(markerPath(cfg.sessionDir, sessionStem), "utf8");
    } catch {
      return; // no marker — normal start
    }
    const marker = decodeMarker(raw);
    if (!marker) {
      clearMarker();
      return;
    }
    if (!restoreIsCompatible(marker, modelId)) {
      ctx.ui.notify(`vitriol-checkpoint: marker is for model ${marker.model}, engine runs ${modelId || "?"} — skip restore`, "warning");
      clearMarker();
      return;
    }
    const r = await slotOp("restore", marker.filename);
    if (r.ok) {
      ctx.ui.notify(`vitriol-checkpoint: restored warm KV from ${marker.filename} (turn ${marker.turn})`, "info");
      clearMarker(); // restored; keep only if engine refused (stale slot file)
    } else {
      ctx.ui.notify(`vitriol-checkpoint: restore failed (${r.error}) — continuing cold`, "warning");
    }
  });

  pi.on("turn_end", async (event) => {
    if (cfg.everyTurns <= 0) return;
    const turn = Number((event as { turnIndex?: number }).turnIndex ?? 0);
    if (turn === 0 || turn % cfg.everyTurns !== 0 || turn === lastSavedTurn) return;
    const filename = turnFilename(turn, sessionStem);
    const r = await slotOp("save", filename);
    if (r.ok) {
      lastSavedTurn = turn;
      writeMarker(filename, turn);
      emitHarnessEvent(harnessEvent("lc-ckpt", "saved", { turn, detail: filename, session: sessionStem }));
    }
  });

  pi.registerTool({
    name: "vitriol_checkpoint",
    label: "Vitriol Checkpoint",
    description:
      "Save or restore the engine KV slot to disk (§2.2). Use action=save before risky bulk " +
      "refactors / long autonomous stretches; action=restore after a crash or to rewind KV to a " +
      "checkpoint (pair with snapshot_rewind turn-N for the code side). Requires server slot_save_path.",
    parameters: Type.Object({
      action: Type.Union([Type.Literal("save"), Type.Literal("restore")]),
      turn: Type.Optional(Type.Number({ description: "Checkpoint turn number (default: current turn)" })),
    }),
    async execute(_id, { action, turn }) {
      const n = turn ?? Math.max(lastSavedTurn, 0);
      const filename = turnFilename(n, sessionStem);
      const r = await slotOp(action, filename);
      if (!r.ok) {
        return { content: [{ type: "text" as const, text: `vitriol-checkpoint ${action} failed: ${r.error}` }], details: {}, isError: true };
      }
      if (action === "save") writeMarker(filename, n);
      else clearMarker();
      emitHarnessEvent(harnessEvent("lc-ckpt", action === "save" ? "saved" : "restored", { turn: n, detail: filename, session: sessionStem }));
      const size = r.nBytes !== undefined ? ` ${(r.nBytes / 1e6).toFixed(1)} MB` : "";
      const tok = r.nTokens !== undefined ? `${r.nTokens} tok` : "state";
      return { content: [{ type: "text" as const, text: `${action}d ${filename}: ${tok}${size}${r.tMs !== undefined ? ` in ${Math.round(r.tMs)} ms` : ""}` }], details: {} };
    },
  });
}
