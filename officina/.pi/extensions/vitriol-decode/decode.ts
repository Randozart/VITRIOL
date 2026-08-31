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
}

const COUNTER_KEYS: Record<string, keyof MetricsCounters> = {
  "llamacpp:prompt_tokens_total": "promptTokens",
  "llamacpp:n_decode_total": "decodeTokens",
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
  return { promptTokens: out.promptTokens, decodeTokens: out.decodeTokens };
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
