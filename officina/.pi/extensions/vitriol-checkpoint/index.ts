import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import {
  mkdirSync,
  readdirSync,
  readFileSync,
  unlinkSync,
  writeFileSync,
  statSync,
} from "node:fs";
import { dirname } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  ckptConfig,
  checkpointPeerGate,
  decodeMarker,
  encodeMarker,
  markerMatchesSession,
  markerPath,
  parseSlotOpResponse,
  overBudgetTurnFiles,
  prunableTurnFiles,
  isTurnFile,
  restoreIsCompatible,
  safeFilename,
  slotActionUrl,
  turnFilename,
  type CkptConfig,
  type SlotOpResult,
} from "./client.ts";
import { readLivePeers } from "../_shared/routing-registry.ts";
import { traceHooks } from "../_shared/hook-trace.ts";
import {
  quarantineRecoveryMarker,
  shouldQuarantineRestore,
} from "./recovery.ts";

// vitriol-checkpoint — scaffold trigger for engine slot save/restore (§2.2,
// step 4). Endpoints exist in this fork (POST /slots/<id>?action=save|restore,
// requires --slot-save-path — set in the profiles). Autosaves every
// TRIS_CKPT_EVERY_TURNS turns (default 10, 0 = manual only), leaves a marker,
// and a new session RESTORES from the marker instead of re-prefilling turn 0.
// Filenames pair with the snapshot ext (turn-N) → rewind code + KV together.
//
// Multi-session guard (I5, 2026-11-11): with live peer sessions on the
// engine (parallel=1, engine-assigned slots), BOTH directions are gated —
// autosave skips (the slot may hold a peer's state) and crash-restore
// skips (restoring would clobber the peer's resident KV). The marker must
// also carry this session's stem-encoded filename.
// Kill switch: TRIS_NO_VITRIOL_CKPT=1. Layer rule: HTTP API only.

/**
 * In-flight autosaves. The `turn_end` handler never awaits them (an engine
 * write must not sit on pi's drain path); tests and diagnostics await this set.
 */
export const pendingAutosaves = new Set<Promise<void>>();

export default function (pi: ExtensionAPI) {
  const cfg = ckptConfig();
  if (!cfg.enabled) return;
  // Opt-in hook timing (2026-11-15): a turn_end save that awaits a busy engine
  // must be visible, not guessed at.
  traceHooks("vitriol-checkpoint", pi);

  let sessionStem = "session";
  let lastSavedTurn = -1;
  let modelId = process.env.TRIS_CKPT_MODEL ?? "";

  // Trim the accumulated backlog at session start (2026-11-21): a box whose
  // saves are rare otherwise keeps a dead-session backlog indefinitely
  // (63 GiB observed). Best effort; the save path re-enforces after each save.
  try {
    enforceTurnBudget("");
  } catch {
    // budget enforcement must never break session start
  }

  /** One engine call; never throws — the checkpoint layer must not break a run. */
  /**
   * Resolve the engine's slot_save_path: env override first, then the
   * launcher config (same resolution the TUI's cache.rs uses — one source
   * of truth on disk). Empty when unset: retention silently skips.
   */
  function slotSaveDir(): string | null {
    const envDir = process.env.VITRIOL_KV_DISK_DIR?.trim();
    if (envDir) return envDir.replace(/^~\/ /, `${process.env.HOME ?? ""}/`);
    try {
      const cfgText = readFileSync(
        `${process.env.HOME}/.vitriol/config`,
        "utf8",
      );
      for (const line of cfgText.split("\n")) {
        const m = line.match(/^\s*slot_save_path\s*=\s*(.+)$/);
        if (m) {
          const v = m[1].trim();
          return v.replace(/^~\//, `${process.env.HOME ?? ""}/`);
        }
      }
    } catch {
      // no config — retention cannot run without a dir
    }
    return null;
  }

  async function slotOp(
    action: "save" | "restore",
    filename: string,
  ): Promise<SlotOpResult> {
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

  /**
   * Retention sweep (2026-11-19 cap breach): turn checkpoints live in the
   * engine's slot_save_path, which the engine's cache_max_gb budget does NOT
   * count (scan matches the cache and slot filename prefixes only —
   * server-task.cpp:2394).
   * The writer therefore enforces its own ring: keep the newest
   * cfg.keepTurns files of THIS session, prune the rest. FS access is the
   * documented exception to HTTP-only sovereignty (same host, dir from
   * ~/.vitriol/config, precedent: TUI cache.rs). Best effort — a failure
   * must never fail the save that triggered it.
   */
  function pruneOldTurns(justSavedTurn: number): void {
    const dir = slotSaveDir();
    if (!dir) return;
    let names: string[];
    try {
      names = readdirSync(dir);
    } catch {
      return;
    }
    for (const name of prunableTurnFiles(
      names,
      sessionStem,
      cfg.keepTurns,
      justSavedTurn,
    )) {
      // Containment: only ever unlink inside the resolved save dir.
      if (name.includes("/") || name.includes("\\")) continue;
      try {
        unlinkSync(`${dir}/${name}`);
      } catch {
        // already gone or unwritable — retention is best effort
      }
    }
  }

  /**
   * Global turn-file budget (2026-11-21): the per-session ring alone let dead
   * sessions hold 3 × ~2.5 GiB each (63 GiB on disk vs the 40 GiB
   * cache_max_gb). After the ring prune, sweep the NEWEST-FIRST-EXEMPT set:
   * delete oldest turn files across all sessions until the total fits
   * cfg.maxTurnGb, always protecting the file just written. Best effort — a
   * failure must never fail the save that triggered it.
   */
  function enforceTurnBudget(justSaved: string): void {
    const dir = slotSaveDir();
    if (!dir) return;
    let names: string[];
    try {
      names = readdirSync(dir);
    } catch {
      return;
    }
    const stats = names.filter(isTurnFile).map((name) => {
      try {
        const st = statSync(`${dir}/${name}`);
        return { name, size: st.size, mtimeMs: st.mtimeMs };
      } catch {
        return { name, size: 0, mtimeMs: 0 };
      }
    });
    for (const name of overBudgetTurnFiles(
      stats,
      cfg.maxTurnGb * 1024 ** 3,
      new Set([justSaved]),
    )) {
      if (name.includes("/") || name.includes("\\")) continue;
      try {
        unlinkSync(`${dir}/${name}`);
      } catch {
        // already gone or unwritable — budget enforcement is best effort
      }
    }
  }

  function writeMarker(filename: string, turn: number): void {
    try {
      const p = markerPath(cfg.sessionDir, sessionStem);
      mkdirSync(dirname(p), { recursive: true });
      writeFileSync(
        p,
        encodeMarker({
          endpoint: cfg.endpoint,
          slot: cfg.slot,
          filename,
          turn,
          model: modelId,
          at: Date.now(),
        }),
      );
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
    const sm = (
      ctx as { sessionManager?: { getSessionFile?: () => string | null } }
    ).sessionManager;
    sessionStem =
      (sm?.getSessionFile?.() ?? "")
        .split("/")
        .pop()
        ?.replace(/\.jsonl$/, "") || "session";
    lastSavedTurn = -1; // a switched-in session must autosave from scratch
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
    if (!markerMatchesSession(marker, sessionStem)) {
      ctx.ui.notify(
        `vitriol-checkpoint: marker file ${marker.filename} is not this session's (${turnFilename(marker.turn, sessionStem)}) — skip restore`,
        "warning",
      );
      clearMarker();
      return;
    }
    const gate = checkpointPeerGate(readLivePeers(process.pid).length);
    if (!gate.allowed) {
      ctx.ui.notify(
        `vitriol-checkpoint: restore skipped — ${gate.reason}`,
        "warning",
      );
      // Keep the marker: this session may restore on a later solo start.
      return;
    }
    if (!restoreIsCompatible(marker, modelId)) {
      ctx.ui.notify(
        `vitriol-checkpoint: marker is for model ${marker.model}, engine runs ${modelId || "?"} — skip restore`,
        "warning",
      );
      clearMarker();
      return;
    }
    const r = await slotOp("restore", marker.filename);
    if (r.ok) {
      ctx.ui.notify(
        `vitriol-checkpoint: restored warm KV from ${marker.filename} (turn ${marker.turn})`,
        "info",
      );
      clearMarker();
    } else {
      const backup = shouldQuarantineRestore(r)
        ? quarantineRecoveryMarker(markerPath(cfg.sessionDir, sessionStem), raw)
        : undefined;
      const recovery = backup
        ? `automatic retry stopped; recovery marker preserved at ${backup}`
        : "recovery marker kept for a later retry";
      ctx.ui.notify(
        `vitriol-checkpoint: restore failed (${r.error}) — continuing cold; ${recovery}`,
        "warning",
      );
    }
  });

  // Turn-end autosave must NEVER sit on pi's event drain path: the engine is
  // single-slot, so a save issued while another session decodes would delay
  // this session's next tool for a whole generation (2026-11-15). Fire and
  // forget, bounded, one in flight.
  let saveInFlight = false;
  pi.on("turn_end", (event) => {
    if (cfg.everyTurns <= 0 || saveInFlight) return;
    const turn = Number((event as { turnIndex?: number }).turnIndex ?? 0);
    if (turn === 0 || turn % cfg.everyTurns !== 0 || turn === lastSavedTurn)
      return;
    saveInFlight = true;
    const promise = saveTurn(turn).finally(() => {
      saveInFlight = false;
      pendingAutosaves.delete(promise);
    });
    pendingAutosaves.add(promise);
  });

  /** Autosave one turn's KV state; never awaited by the actor, never throws. */
  async function saveTurn(turn: number): Promise<void> {
    const gate = checkpointPeerGate(readLivePeers(process.pid).length);
    if (!gate.allowed) {
      emitHarnessEvent(
        harnessEvent("lc-ckpt", "skipped", {
          turn,
          detail: gate.reason,
          session: sessionStem,
        }),
      );
      return;
    }
    const filename = turnFilename(turn, sessionStem);
    const r = await slotOp("save", filename);
    if (!r.ok) return;
    lastSavedTurn = turn;
    writeMarker(filename, turn);
    pruneOldTurns(turn);
    enforceTurnBudget(filename);
    emitHarnessEvent(
      harnessEvent("lc-ckpt", "saved", {
        turn,
        detail: filename,
        session: sessionStem,
      }),
    );
  }

  pi.registerTool({
    name: "vitriol_checkpoint",
    label: "Vitriol Checkpoint",
    description:
      "Save or restore the engine KV slot to disk (§2.2). Use action=save before risky bulk " +
      "refactors / long autonomous stretches; action=restore after a crash or to rewind KV to a " +
      "checkpoint (pair with snapshot_rewind turn-N for the code side). Requires server slot_save_path.",
    parameters: Type.Object({
      action: Type.Union([Type.Literal("save"), Type.Literal("restore")]),
      turn: Type.Optional(
        Type.Number({
          description: "Checkpoint turn number (default: current turn)",
        }),
      ),
    }),
    async execute(_id, { action, turn }) {
      const n = turn ?? Math.max(lastSavedTurn, 0);
      const filename = turnFilename(n, sessionStem);
      const r = await slotOp(action, filename);
      if (!r.ok) {
        return {
          content: [
            {
              type: "text" as const,
              text: `vitriol-checkpoint ${action} failed: ${r.error}`,
            },
          ],
          details: {},
          isError: true,
        };
      }
      if (action === "save") writeMarker(filename, n);
      else clearMarker();
      emitHarnessEvent(
        harnessEvent("lc-ckpt", action === "save" ? "saved" : "restored", {
          turn: n,
          detail: filename,
          session: sessionStem,
        }),
      );
      const size =
        r.nBytes !== undefined ? ` ${(r.nBytes / 1e6).toFixed(1)} MB` : "";
      const tok = r.nTokens !== undefined ? `${r.nTokens} tok` : "state";
      return {
        content: [
          {
            type: "text" as const,
            text: `${action}d ${filename}: ${tok}${size}${r.tMs !== undefined ? ` in ${Math.round(r.tMs)} ms` : ""}`,
          },
        ],
        details: {},
      };
    },
  });
}
