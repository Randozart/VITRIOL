// pipeline rules (2026-11-19) — PURE decision logic for the PIPELINE mode's
// confirm gate. Mirrors grind/rules.ts: a directive alone cannot enforce the
// owner-confirmation phase boundary, so the gate is a marker the model must
// emit EXACTLY once the owner confirms, and this module decides whether the
// pipeline may proceed past research.
//
// Invariant I8 (theme-fire-overhaul plan): PIPELINE never implements without
// the owner's explicit plan confirmation. The gate is fail-closed: an absent,
// garbled, or stale marker means "still in research".

export type GateDecision =
  { action: "await-confirmation" } | { action: "proceed"; confirmedAt: string };

/** The exact marker the PIPELINE directive requires (verbatim line). */
export const PLAN_CONFIRMED_MARKER = "PIPELINE: PLAN CONFIRMED";

/**
 * Is the gate satisfied? STRICT: the marker must be a line on its own —
 * inline mentions in prose do not count (a research note quoting the marker
 * must not open the gate).
 */
export function gateOpen(lastAssistantText: string): boolean {
  const lines = String(lastAssistantText ?? "")
    .split(/\r?\n/)
    .map((l) => l.trim());
  return lines.includes(PLAN_CONFIRMED_MARKER);
}

/**
 * What should the pipeline loop do at an agent_end while the gate is
 * unresolved? Fail-closed: anything but an open gate keeps waiting for the
 * owner. `reconfirmations` counts how many times the model has been reminded
 * to stop and ask — the loop uses it to avoid nagging more than twice.
 */
export function gateDecision(
  lastAssistantText: string,
  remindersSent: number,
): GateDecision {
  if (gateOpen(lastAssistantText)) {
    return { action: "proceed", confirmedAt: PLAN_CONFIRMED_MARKER };
  }
  void remindersSent; // reserved for the nag-cap; awaiting owner is always safe
  return { action: "await-confirmation" };
}

/**
 * Once the pipeline is past the gate (plan written, grind running), the
 * GRIND markers govern completion. Re-export the shapes here so the pipeline
 * loop has one import site. (grind/rules.ts owns the grammar.)
 */
export type { Marker } from "../grind/rules.ts";
export { parseMarker } from "../grind/rules.ts";
