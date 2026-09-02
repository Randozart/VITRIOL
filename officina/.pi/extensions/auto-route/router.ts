// PROVENANCE: inspiration — intel/intel-ai-builder (Apache-2.0), Auto Route's
// three execution tiers (local PC / edge / cloud) and the user-tunable
// quality-vs-cost threshold documented in the SuperClaw README and Intel
// newsroom article. The route-table shape, threshold formula, and mode
// machinery are original VITRIOL work; no Intel code was seen or copied.
//
// router — pure (classification × threshold) → model-tier lookup. No I/O.
//
// Tiers:
//   local-sm  — small local model (fast, weak): simple/failed-privacy turns
//   local-lg  — flagship local model: the daily driver
//   cloud     — ascensusd escalation: hard + safe turns only
//
// The threshold is the Auto-Route-style quality-cost knob:
//   0.0 → everything upgrades (max quality, cloud whenever allowed)
//   0.5 → balanced default
//   1.0 → almost nothing escalates (max savings/privacy)
// Formula: effective = complexity * (1 - t) + t * 0.3, so the threshold both
// dampens complexity and adds a floor that keeps trivial turns trivial.

export type Tier = "local-sm" | "local-lg" | "cloud";
export type PrivacyClass = "safe" | "sensitive" | "confidential";
export type RouteMode = "suggest" | "auto" | "off";

export interface RouteDecision {
  tier: Tier;
  /** Why — human-readable, shown in the status line / suggest prompt. */
  reason: string;
  /** Post-threshold complexity the decision was made on. */
  effective: number;
}

/** Clamp + default for the threshold knob. */
export function resolveThreshold(raw: string | undefined): number {
  if (raw === undefined || raw.trim() === "") return 0.5;
  const n = Number(raw);
  if (!Number.isFinite(n)) return 0.5;
  return Math.min(1, Math.max(0, n));
}

export function resolveMode(raw: string | undefined): RouteMode {
  return raw === "auto" || raw === "suggest" || raw === "off" ? raw : "suggest";
}

/** Display-only dampening of complexity for the status line. */
export function effectiveComplexity(complexity: number, threshold: number): number {
  return complexity * (1 - threshold) + threshold * 0.3;
}

/**
 * Route one classified turn.
 *
 * Privacy dominates: sensitive turns never reach cloud; confidential turns
 * additionally downgrade to local-sm (small model, minimal surface). Among
 * safe turns the complexity decides against threshold-shifted cutoffs:
 *   cloud cutoff = 0.30 + 0.70·t  (t=0: anything non-trivial escalates;
 *                                  t=1: only certainty reaches cloud)
 *   small cutoff = 0.15 + 0.15·t
 */
export function route(
  complexity: number,
  privacy: PrivacyClass,
  threshold: number,
): RouteDecision {
  const cloudCutoff = 0.3 + 0.7 * threshold;
  const smallCutoff = 0.15 + 0.15 * threshold;

  if (privacy === "confidential") {
    return { tier: "local-sm", reason: "confidential — local small model only", effective: complexity };
  }
  if (privacy === "sensitive") {
    return {
      tier: complexity >= smallCutoff ? "local-lg" : "local-sm",
      reason: "sensitive — stays local",
      effective: complexity,
    };
  }
  if (complexity >= cloudCutoff) {
    return { tier: "cloud", reason: `complexity ${complexity.toFixed(2)} ≥ ${cloudCutoff.toFixed(2)} — ascensusd`, effective: complexity };
  }
  if (complexity >= smallCutoff) {
    return { tier: "local-lg", reason: `complexity ${complexity.toFixed(2)} — flagship`, effective: complexity };
  }
  return { tier: "local-sm", reason: `complexity ${complexity.toFixed(2)} — small model`, effective: complexity };
}
