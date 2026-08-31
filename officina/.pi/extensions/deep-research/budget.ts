// Budget allocation for the deep-research agent tree.
//
// The user picks a cap M (1-10) at the start of a run. M is the TOTAL budget for
// the research tree — the lead (decompose) agent plus the research waves plus the
// gap-fill wave. The Scope clarifier / brief-writer and the final Writer are
// fixed reasoning steps and are NOT counted in M.
//
// Shape (mirrors the reference "1 lead → 3 research → 1-2 gap-fill", scaled):
//   reserve = clamp(round((M-1) * 0.3), 0, 3)   // held back for gap-fill (wave 2)
//   wave1   = (M - 1) - reserve                  // lead is the other 1
//
//   M=10 → lead 1 + wave1 6 + reserve ≤3
//   M=6  → lead 1 + wave1 3 + reserve ≤2   (the reference default shape)
//   M=4  → lead 1 + wave1 2 + reserve ≤1
//   M=2  → lead 1 + wave1 1 + reserve 0
//   M=1  → single agent, no fan-out, no separate lead
//
// Serial by default (LITTLE_CODER_SUBCODER_CONCURRENCY=1), so a large M is a long
// sequential ride — the user opts into that when they raise the cap.

export const MIN_AGENTS = 1;
export const MAX_AGENTS = 10;

export interface Budget {
  /** The clamped cap the rest of the tree is sized from (1..10). */
  cap: number;
  /** True when cap ≤ 1: one agent researches the whole brief; no lead, no waves. */
  single: boolean;
  /** Research subagents dispatched in wave 1 (the lead is separate). */
  wave1: number;
  /** Upper bound on gap-fill subagents for wave 2; the actual count is decided at runtime. */
  reserve: number;
}

export function clampAgents(n: number): number {
  if (!Number.isFinite(n)) return MAX_AGENTS;
  return Math.max(MIN_AGENTS, Math.min(MAX_AGENTS, Math.floor(n)));
}

export function allocateBudget(maxAgents: number): Budget {
  const cap = clampAgents(maxAgents);
  if (cap <= 1) {
    return { cap: 1, single: true, wave1: 0, reserve: 0 };
  }
  const reserve = Math.max(0, Math.min(3, Math.round((cap - 1) * 0.3)));
  const wave1 = cap - 1 - reserve;
  return { cap, single: false, wave1, reserve };
}
