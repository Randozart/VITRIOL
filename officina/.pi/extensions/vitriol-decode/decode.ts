// Pure decode-telemetry math for the vitriol-decode widget (testable).
//
// First-Party Mandate standing requirement (AGENTS.md 2026-08-31): the
// cockpit shows live decode progress — streamed tokens, t/s, slot state —
// so the VITRIOL TUI is never needed to see agent progress.
// Counter names verified against THIS fork build live 2026-08-29
// (tris_lib.metrics_snapshot): llamacpp:prompt_tokens_total,
// llamacpp:n_decode_total. /slots may carry no per-slot n_kv on this build
// (engine telemetry gap) — the widget renders what exists, never guesses.

export interface MetricsCounters {
  promptTokens: number;
  decodeTokens: number;
  /** VITRIOL sparse-KV: cells ejected since boot (0 on stock engines). */
  ejected?: number;
}

const COUNTER_KEYS: Record<string, keyof MetricsCounters> = {
  "llamacpp:prompt_tokens_total": "promptTokens",
  "llamacpp:n_decode_total": "decodeTokens",
  "llamacpp:kv_ejected_total": "ejected",
};

export function parseMetrics(text: string): MetricsCounters | null {
  const out: Partial<MetricsCounters> = {};
  for (const line of text.split("\n")) {
    const [key, val] = line.trim().split(/\s+/);
    const field = COUNTER_KEYS[key];
    if (!field) continue;
    const n = Number(val);
    if (!Number.isFinite(n)) return null;
    out[field] = n;
  }
  if (out.promptTokens === undefined || out.decodeTokens === undefined) return null;
  return { promptTokens: out.promptTokens, decodeTokens: out.decodeTokens, ejected: out.ejected ?? 0 };
}

/**
 * Parse /v1/models for the engine's loaded model id. Prefers the OpenAI
 * shape (`data[0].id`), falls back to the ollama-ish shape
 * (`models[0].model`). Empty string when unparseable.
 */
export function parseLoadedModel(text: string): string {
  try {
    const j = JSON.parse(text) as {
      data?: Array<{ id?: string }>;
      models?: Array<{ model?: string }>;
    };
    return j.data?.[0]?.id ?? j.models?.[0]?.model ?? "";
  } catch {
    return "";
  }
}

/**
 * Parse /props for the loaded model's file path (top-level `model_path`).
 * Empty string when absent/unparseable.
 */
export function parseModelPath(text: string): string {
  try {
    const j = JSON.parse(text) as { model_path?: string };
    return j.model_path ?? "";
  } catch {
    return "";
  }
}

export interface DecodeDelta {
  tps: number;
  tokens: number;
}

export function counterDelta(
  before: MetricsCounters | null,
  after: MetricsCounters,
  seconds: number,
): DecodeDelta {
  if (!before || seconds <= 0) return { tps: 0, tokens: 0 };
  const tokens = Math.max(0, after.decodeTokens - before.decodeTokens);
  return { tps: tokens / seconds, tokens };
}

// A 24-char progress bar; `fraction` in [0,1] clamped. Pretty, blocky, no
// external dependency: "█" filled, "░" empty, per Crush-style TUI aesthetics.
export function renderBar(fraction: number, width = 24): string {
  const f = Math.min(1, Math.max(0, fraction));
  const filled = Math.round(f * width);
  return "█".repeat(filled) + "░".repeat(width - filled);
}

export function fmtRate(tps: number): string {
  return tps >= 10 ? tps.toFixed(1) : tps >= 0.1 ? tps.toFixed(2) : "0";
}

export function fmtTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(Math.floor(n));
}

export interface SlotInfo {
  id: number;
  busy: boolean;
}

// Live engine truth (verified 2026-08-31): the llama-server slot schema's
// busy flag is "is_processing"; "busy" kept as a fallback alias.
export function parseSlots(text: string): SlotInfo[] {
  try {
    const json = JSON.parse(text) as unknown;
    const arr = Array.isArray(json) ? json : ((json as { slots?: unknown }).slots ?? []);
    const out: SlotInfo[] = [];
    for (const s of arr as Array<Record<string, unknown>>) {
      if (typeof s === "object" && s !== null) {
        out.push({ id: Number(s.id ?? -1), busy: Boolean(s.is_processing) || Boolean(s.busy) });
      }
    }
    return out;
  } catch {
    return [];
  }
}

// Busy truth is TWO-source: slot flags AND token movement. If tokens are
// flowing the engine is not idle, whatever a single slot/GPU reports
// (2026-08-31 bugfix: widget said "idle" during dual-GPU decode).
export function busySlots(slots: SlotInfo[], deltaTokens: number): number {
  const flagged = slots.filter((s) => s.busy).length;
  if (flagged > 0 || deltaTokens > 0) return Math.max(1, flagged);
  return 0;
}

// ── Composer-fire load math (owner request 2026-09-02) ───────────────────
// The Rust TUI's braille flames rise from the prompt box with GPU load.
// Pure math here (testable); engine.ts owns the nvidia-smi poll that feeds
// the numbers, vitriol-decode/index.ts the widget emission.

/**
 * Fire load in [0,1] from ONE GPU's nvidia-smi numbers.
 * Primary: power.draw over power.limit, with the idle baseline subtracted —
 * a desktop Pascal/Ada card idles around a quarter of its power cap, and
 * that must read as "no fire". Utilization is the secondary signal
 * (true 0 at idle), discounted slightly so it never outruns the power arc.
 */
export function gpuFireLoad(powerW: number, limitW: number, utilPct: number): number {
  if (!(limitW > 0) || !(powerW >= 0)) return 0;
  const IDLE_FRAC = 0.25;
  const byPower = (powerW / limitW - IDLE_FRAC) / (1 - IDLE_FRAC);
  const byUtil = utilPct >= 0 ? (utilPct / 100) * 0.95 : 0;
  const raw = Math.max(byPower, byUtil);
  // Dead zone: desktop background tasks (idle util spikes, tiny draws)
  // must not read as embers.
  return raw < 0.06 ? 0 : Math.min(1, raw);
}

export interface FireLoadInput {
  up: boolean;
  busy: number;
  slotCount: number;
  tps: number;
  ingestTps: number;
  /** max-across-GPU load from the nvidia-smi poll; null when unavailable */
  gpuLoad: number | null;
}

/**
 * Flame intensity in [0,1]: real GPU draw when known, otherwise an
 * activity proxy (step-like by nature — decode t/s is flat while decoding).
 */
export function fireLoad(snap: FireLoadInput): number {
  if (!snap.up) return 0;
  if (snap.gpuLoad != null) return Math.min(1, Math.max(0, snap.gpuLoad));
  const total = Math.max(snap.slotCount, snap.busy, 1);
  return Math.min(
    1,
    Math.max(
      snap.busy > 0 ? 0.55 : 0,
      Math.min(1, snap.tps / 15),
      Math.min(1, snap.ingestTps / 500),
    ),
  );
}
