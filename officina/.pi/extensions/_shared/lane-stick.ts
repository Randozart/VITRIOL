// Lane continuation stick — one pure rule for "keep the current physical model
// on a continuation turn?", shared by both virtual lanes (2026-11-21).
//
// WHY this exists: both lanes short-circuited every `continuation` with
// `return { model: previous }`. That is right for a tool loop on the lane's
// own tier (it keeps weights warm), but it was UNCONDITIONAL — so one local
// turn pinned the whole loop to local: Nimbus (cloud-first) lost its priority
// for the rest of the session, and Lapis kept re-using a contended slot,
// bypassing the 2026-11-20 admission probe entirely.
//
// The rule is deliberately conservative: a stick is the default; the two
// escapes are (a) the lane's own tier preference is violated and (b) there is
// capacity to honour it. When there is nowhere better to go, stick — a swap
// that lands on a cooling/exhausted pool would strand the turn.
//
// PROVENANCE: original work, this repo.

export type Lane = "lapis" | "nimbus";

/** Inputs are the minimum facts needed; the caller resolves them lazily. */
export interface StickInput {
  lane: Lane;
  /** The previous turn's physical model was the local engine. */
  previousIsLocal: boolean;
  /** Fresh observation that the single local slot is held by someone else. */
  contended: boolean;
  /** Some cloud lane can take this prompt right now (fits + budget). */
  cloudFits: boolean;
}

export interface StickDecision {
  stick: boolean;
  /** Why the lane re-decides instead of sticking (empty when sticking). */
  reason: string;
}

/**
 * Should this continuation keep the previous physical model?
 *
 * Table (pinned by lane-stick.test.ts):
 *   nimbus + cloud previous                  → stick
 *   nimbus + local previous + cloud capacity → re-decide
 *   nimbus + local previous, no capacity     → stick
 *   lapis  + cloud previous                  → stick (warm cache)
 *   lapis  + local previous, uncontended     → stick
 *   lapis  + local previous, contended, capacity → re-decide
 *   lapis  + local previous, contended, no capacity → stick
 */
export function stickDecision(input: StickInput): StickDecision {
  if (!input.previousIsLocal) return { stick: true, reason: "" };

  if (input.lane === "nimbus") {
    // Cloud-first is the operator's explicit priority: a local leftover from
    // one turn must not outlive the reason it was chosen.
    return input.cloudFits
      ? { stick: false, reason: "nimbus: cloud-first — re-decide off local" }
      : { stick: true, reason: "" };
  }

  // Lapis is local-first: only contention justifies leaving the stone.
  if (input.contended && input.cloudFits) {
    return { stick: false, reason: "lapis: local slot contended — re-decide" };
  }
  return { stick: true, reason: "" };
}
