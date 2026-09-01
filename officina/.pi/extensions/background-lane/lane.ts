// background-lane — pure module: config, gate decision, prompt construction,
// card rendering, patch coalescing. Execution (HTTP, timers) is injected or
// lives in index.ts so this file stays unit-testable.
//
// Design contract (plan: .opencode/plans/dual-slot-background-lane-2026-08-31.md):
// bounded input (patch + local context) → compact card (≤ ~500 tokens) →
// queue → main agent pulls. Jobs needing the conversation transcript are
// refused by construction — index.ts never sees one.
//
// Provenance: original work, this repo (First-Party Mandate).

export type LaneGate = "always" | "idle";

export interface LaneConfig {
  enabled: boolean;
  /**
   * When to launch jobs:
   *  - "always": launch as soon as patches exist, even during foreground
   *    decode. On the dual-GPU tensor split the two slots interleave and the
   *    aggregate rate rises (measured 1.51x, bench-dual-slot.py 2026-09-01);
   *    foreground decode dips ~20% while a job runs.
   *  - "idle": only between foreground turns (the conservative original).
   */
  gate: LaneGate;
  /** Base URL of the VITRIOL engine (llama-server). */
  base: string;
  /** Engine must be idle this long (ms) before a job launches. */
  idleMs: number;
  /** Minimum chars of accumulated patch before a review is worth a job. */
  minPatchChars: number;
  /** Max chars of patch sent to the reviewer. */
  maxPatchChars: number;
  /** Max tokens for the reviewer's response. */
  maxTokens: number;
  /** Max chars of card content injected into the main context. */
  cardBudgetChars: number;
  /** Per-request HTTP timeout. */
  timeoutMs: number;
  /** Slot preference for the lane (engine default routing if undefined). */
  slotId?: number;
}

export function laneConfig(env: NodeJS.ProcessEnv = process.env): LaneConfig {
  const n = (v: string | undefined, d: number) => {
    const x = v === undefined ? NaN : Number(v);
    return Number.isFinite(x) ? x : d;
  };
  const slotRaw = env.OFFICINA_BG_SLOT;
  const gate = env.OFFICINA_BG_GATE === "idle" ? "idle" : "always";
  return {
    enabled: env.OFFICINA_NO_BACKGROUND !== "1",
    gate,
    base: (env.VITRIOL_BASE_URL || "http://127.0.0.1:8279").replace(/\/$/, ""),
    idleMs: n(env.OFFICINA_BG_IDLE_MS, 4000),
    minPatchChars: n(env.OFFICINA_BG_MIN_PATCH, 200),
    maxPatchChars: n(env.OFFICINA_BG_MAX_PATCH, 8000),
    maxTokens: n(env.OFFICINA_BG_MAX_TOKENS, 320),
    cardBudgetChars: n(env.OFFICINA_BG_CARD_BUDGET, 700),
    timeoutMs: n(env.OFFICINA_BG_TIMEOUT_MS, 120_000),
    slotId: slotRaw === undefined || slotRaw === "" ? undefined : n(slotRaw, 0),
  };
}

/**
 * Shared reviewer prefix — byte-stable across jobs so llama.cpp's prompt
 * cache gives the lane KV reuse on every job after the first. The diff is
 * appended AFTER this prefix, never inside it. Pure.
 */
export const REVIEWER_PREFIX = `You are a code reviewer. You are given a unified diff of edits an agent just made to a repository. Review ONLY the diff: report real bugs, missed edge cases, and inconsistencies with the surrounding code shown in the diff context lines. Do not comment on style. If the diff is clean, reply exactly: CLEAN. Be terse: at most 5 bullet points, one line each.

DIFF:
`;

export function buildReviewPrompt(patch: string, cfg: LaneConfig): string {
  const clipped = patch.length > cfg.maxPatchChars ? patch.slice(0, cfg.maxPatchChars) + "\n… [truncated]" : patch;
  return REVIEWER_PREFIX + clipped;
}

/** Response is a findings card, or null when the reviewer said CLEAN. Pure. */
export function renderCard(file: string, response: string, cfg: LaneConfig): string | null {
  const text = response.trim();
  if (!text || /^clean\b\.?$/i.test(text)) return null;
  const capped = text.length > cfg.cardBudgetChars ? text.slice(0, cfg.cardBudgetChars) + "…" : text;
  return `[background review · ${file}]\n${capped}`;
}

/**
 * Gate decision. "always" launches whenever the engine is up and a job is
 * pending (concurrent decode — owner preference 2026-09-01, the dual-GPU
 * split interleaves slot work with only a ~20% per-slot dip). "idle" is the
 * conservative mode: wait for a fully idle engine for idleMs. Pure.
 */
export function shouldLaunch(
  snapshot: { up: boolean; busy: number; delta: { tps: number } },
  pendingChars: number,
  idleSince: number | null,
  now: number,
  cfg: LaneConfig,
): boolean {
  if (pendingChars < cfg.minPatchChars) return false;
  if (!snapshot.up) return false;
  if (cfg.gate === "always") return true;
  if (snapshot.busy > 0 || snapshot.delta.tps > 0) return false;
  if (idleSince === null) return false;
  return now - idleSince >= cfg.idleMs;
}

/** Coalesced patch accumulator: aggregates edit patches across a turn. */
export class PatchSink {
  private parts: string[] = [];
  private files = new Set<string>();
  private chars = 0;

  add(file: string, patch: string | undefined): void {
    if (!patch) return;
    if (this.files.has(file)) return; // one review window per file per batch
    this.files.add(file);
    this.parts.push(`--- ${file} ---\n${patch}`);
    this.chars += patch.length;
  }

  /** Drain the sink; returns the combined patch, or null if too small. */
  drain(minChars: number, maxChars: number): string | null {
    const combined = this.parts.join("\n");
    this.parts = [];
    this.files = new Set();
    this.chars = 0;
    if (combined.length < minChars) return null;
    return combined.length > maxChars ? combined.slice(0, maxChars) + "\n… [truncated]" : combined;
  }

  get size(): number {
    return this.chars;
  }
}
