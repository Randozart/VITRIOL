// vitriol-checkpoint — KV slot save/restore from the scaffold (REPORT-02 §2.2,
// step 4). The engine endpoints EXIST in this llama.cpp fork:
//   POST /slots/<id>?action=save    {"filename": "x.bin"}
//   POST /slots/<id>?action=restore {"filename": "x.bin"}
// (server-context.cpp SERVER_TASK_TYPE_SLOT_SAVE/LOAD; requires the server to
// run with --slot-save-path, which the vitriol launcher wires from
// server.slot_save_path). lull_slot_persist.py uses the same calls — this
// extension is the SCALED-side trigger: save before risky ops, restore after
// a crash instead of re-prefilling from turn 0.
//
// Pairing with the snapshot ext (step 10, git-per-turn): files are named
// turn-<n>.bin so `git ref turns/<n>` + `restore turn-<n>.bin` rewind code
// AND context to the same turn (the §2.2 pairing, now wired end to end).
//
// Layer sovereignty (Rule 2): HTTP API only — the ONLY engine surface.
// Kill switch: TRIS_NO_VITRIOL_CKPT=1. Autosave cadence: TRIS_CKPT_EVERY_TURNS.


/** Checkpoint extension config. */
export interface CkptConfig {
  enabled: boolean;
  endpoint: string; // engine base, /v1 stripped
  slot: number;
  everyTurns: number; // 0 = autosave off (manual tool still available)
  sessionDir: string; // crash marker dir (workspace-relative)
}

export function ckptConfig(env: NodeJS.ProcessEnv = process.env): CkptConfig {
  const base = (env.VITRIOL_BASE_URL?.trim() || "http://127.0.0.1:8279/v1")
    .replace(/\/+$/, "")
    .replace(/\/v1$/, "");
  const every = Number(env.TRIS_CKPT_EVERY_TURNS ?? "10");
  return {
    enabled: env.TRIS_NO_VITRIOL_CKPT !== "1",
    endpoint: base || "http://127.0.0.1:8279",
    slot: Number.isFinite(Number(env.TRIS_CKPT_SLOT)) ? Math.max(0, Math.floor(Number(env.TRIS_CKPT_SLOT ?? 0))) : 0,
    everyTurns: Number.isFinite(every) && every >= 0 ? Math.floor(every) : 10,
    sessionDir: env.TRIS_CKPT_DIR || ".pi/ckpt",
  };
}

/** Shared filename convention (single definition: _shared/turnkeys.ts). */
export { turnFilename } from "../_shared/turnkeys.ts";

/** URL for one slot action (validate shape before the request). */
export function slotActionUrl(cfg: CkptConfig, action: "save" | "restore"): string {
  if (action !== "save" && action !== "restore") throw new Error(`bad action: ${action}`);
  return `${cfg.endpoint}/slots/${cfg.slot}?action=${action}`;
}

/** Sanitize a filename (no traversal: slashes and dot-runs cannot escape slot-save-path). */
export function safeFilename(name: string): string {
  const clean = name.replace(/[^a-zA-Z0-9._-]/g, "_").replace(/\.{2,}/g, "_");
  return clean.endsWith(".bin") ? clean : `${clean}.bin`;
}

/** Parsed outcome of a save/restore call. */
export interface SlotOpResult {
  ok: boolean;
  status: number;
  filename?: string;
  nTokens?: number;
  nBytes?: number;
  tMs?: number;
  error?: string;
}

/** Pull the engine's error message out of either error body shape. Pure. */
export function engineErrorMessage(status: number, body: unknown): string {
  if (typeof body !== "object" || body === null || !("error" in body)) return `HTTP ${status}`;
  const err = (body as { error: unknown }).error;
  if (typeof err === "string") return err || `HTTP ${status}`;
  const msg = (err as { message?: unknown } | null)?.message;
  return typeof msg === "string" && msg ? msg : `HTTP ${status}`;
}

/**
 * Interpret the engine's response. Exact shapes (verified against
 * server-task.cpp:1901-1923 to_json on a live 2026-08-29 engine):
 *   save:    {"id_slot":0,"filename":"f","n_saved":53,"n_written":53181480,"timings":{"save_ms":42.5}}
 *   restore: {...,"n_restored":N,"n_read":B,"timings":{"restore_ms":T}}
 *   error:   {"error":{"code":400,"message":"...","type":"..."}}
 * Unknown 2xx is treated as ok with no numbers — never guess values.
 */
export function parseSlotOpResponse(status: number, body: unknown): SlotOpResult {
  if (status >= 400) return { ok: false, status, error: engineErrorMessage(status, body) };
  const b = (body ?? {}) as Record<string, unknown> & { timings?: Record<string, unknown> };
  const num = (v: unknown): number | undefined => (typeof v === "number" ? v : undefined);
  return {
    ok: true,
    status,
    filename: typeof b.filename === "string" ? b.filename : undefined,
    nTokens: num(b.n_saved) ?? num(b.n_restored),
    nBytes: num(b.n_written) ?? num(b.n_read),
    tMs: num(b.timings?.save_ms) ?? num(b.timings?.restore_ms),
  };
}

/** Marker file path for crash-recovery (session stem keyed, traversal-safe). */
export function markerPath(dir: string, sessionStem: string): string {
  const clean = sessionStem.replace(/[^a-zA-Z0-9._-]/g, "_").replace(/\.{2,}/g, "_");
  return `${dir}/${clean}.json`;
}

/** Marker payload — written after each autosave so a restart knows what to restore. */
export interface CkptMarker {
  endpoint: string;
  slot: number;
  filename: string;
  turn: number;
  model: string;
  at: number;
}

export function encodeMarker(m: CkptMarker): string {
  return JSON.stringify(m, null, 2);
}

export function decodeMarker(text: string): CkptMarker | null {
  try {
    const m = JSON.parse(text) as CkptMarker;
    if (typeof m.filename !== "string" || typeof m.turn !== "number") return null;
    return m;
  } catch {
    return null;
  }
}

/** True when a marker should be restored: same engine slot + same model. */
export function restoreIsCompatible(m: CkptMarker, currentModel: string): boolean {
  if (!currentModel) return true; // unknown current model — engine validates
  return m.model === currentModel; // restoring into a different model = corruption
}

