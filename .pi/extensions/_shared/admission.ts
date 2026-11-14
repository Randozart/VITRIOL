// Lane admission (2026-11-14) — PURE, parity-tested against vitriol-core.
//
// Mirrors `vitriol_core::{effective_window, admit}`. The router uses this to
// decide whether a cloud lane may receive a request, using the smallest known
// window (catalog, learned provider limit, owner safe-byte ceiling).

export interface LaneAdmissionState {
  invalid?: boolean;
  cooling?: boolean;
  ownerPaused?: boolean;
}

export type Admission = "admit" | "invalid" | "cooling" | "owner-paused" | "no-fit";

/** Smallest non-zero window; undefined when all inputs are unknown. */
export function effectiveWindow(
  declared: number,
  learned?: number | null,
  safeBytes?: number | null,
): number | undefined {
  const safeTokens = safeBytes && safeBytes > 0 ? Math.floor(safeBytes / 4) : undefined;
  let window = Number(declared) || 0;
  for (const value of [learned, safeTokens]) {
    if (!value || value <= 0) continue;
    if (window === 0 || value < window) window = value;
  }
  return window > 0 ? window : undefined;
}

export interface AdmissionInput {
  promptTokens: number;
  reserveTokens: number;
  lane: LaneAdmissionState;
  declared: number;
  learned?: number | null;
  safeBytes?: number | null;
}

export function admitLane(input: AdmissionInput): Admission {
  if (input.lane.invalid) return "invalid";
  if (input.lane.cooling) return "cooling";
  if (input.lane.ownerPaused) return "owner-paused";
  const window = effectiveWindow(input.declared, input.learned, input.safeBytes);
  if (window !== undefined && input.promptTokens + input.reserveTokens < window) {
    return "admit";
  }
  return "no-fit";
}
