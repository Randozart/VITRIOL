// Turn-key convention — the naming contract that pairs CODE snapshots with
// KV checkpoints so `/rewind N` can restore both from one number. Owned here
// (not per-extension) because it is precisely the kind of convention that
// rots when duplicated: snapshot (refs), vitriol-checkpoint (slot files) and
// rewind (pairing) all import this single definition.

export const DEFAULT_REF_PREFIX = "refs/trismegistus/turns";

/** Git ref for one turn snapshot. */
export function turnRef(turn: number, prefix: string = DEFAULT_REF_PREFIX): string {
  return `${prefix}/${turn}`;
}

/** Engine slot checkpoint filename for one turn (+ session stem). */
export function turnFilename(turn: number, sessionStem = "session"): string {
  const safe = sessionStem.replace(/[^a-zA-Z0-9._-]/g, "_");
  return `${safe}-turn-${turn}.bin`;
}
