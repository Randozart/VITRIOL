// mode-return — the return-stack for steer/inquire bounce-back (2026-11-21).
//
// Owner request: steer and INQUIRE should jump back to the work mode they
// interrupted, after answering — and never interrupt research/pipeline/grind
// mechanics. This module owns the pure decision: given the facts file's
// returnTo record and the current mode, should we bounce back, and to where?
//
// Persistence lives in agent-mode.json (the file the agent-mode extension
// already writes) — module state does not survive jiti boundaries, the file
// does (I7).
//
// The rules (plan mode-return-compact-loop-2026-11-21.md):
//   - entering steer/inquire FROM a work mode records returnTo = that work
//     mode (I2)
//   - steer → inquire (and back) does NOT overwrite returnTo (I2: the
//     interloper pair shares one origin)
//   - the operator pressing ESC or picking a work mode clears returnTo (I1:
//     bounce-back never fights the operator)
//   - agent_settled while in steer/inquire with a live returnTo → bounce back

/** Modes that do real work and are worth returning to. */
export const WORK_MODES: ReadonlySet<string> = new Set([
  "build",
  "grind",
  "plan",
  "pipeline",
  "research",
]);

/** The interloper pair: steer and inquire. */
export const INTERRUPT_MODES: ReadonlySet<string> = new Set([
  "steer",
  "inquire",
]);

export function isWorkMode(mode: string | null | undefined): boolean {
  return mode != null && WORK_MODES.has(mode);
}

export function isInterruptMode(mode: string | null | undefined): boolean {
  return mode != null && INTERRUPT_MODES.has(mode);
}

/**
 * After a mode switch TO `newMode`: what should returnTo become?
 *  - switching to a work mode            → clear (operator picked work; I1)
 *  - switching to steer/inquire FROM a work mode → record the origin (I2)
 *  - switching steer↔inquire            → keep the existing origin (I2)
 *  - anything else                      → keep
 */
export function nextReturnTo(
  newMode: string,
  previousMode: string | undefined,
  currentReturnTo: string | null,
): string | null {
  if (isWorkMode(newMode)) return null;
  if (isInterruptMode(newMode) && isWorkMode(previousMode)) {
    return previousMode ?? null;
  }
  return currentReturnTo;
}

/**
 * On agent_settled while sitting in an interrupt mode: bounce back?
 * Only when a returnTo is live (I1: no record, no bounce — the operator may
 * have ESC'd precisely to stay here).
 */
export function bounceTarget(
  currentMode: string,
  returnTo: string | null,
): string | null {
  if (!isInterruptMode(currentMode)) return null;
  return returnTo != null && WORK_MODES.has(returnTo) ? returnTo : null;
}
