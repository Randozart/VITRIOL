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

/**
 * Git ref for one SESSION-namespaced turn snapshot (2026-11-11, I6 of
 * .opencode/plans/officina-multisession-hardening-2026-11-11.md): turn
 * indexes are per-session, so two concurrent sessions both writing
 * `turns/<n>` clobbered each other's refs and a rewind could restore a
 * FOREIGN tree. New writes go to `turns/<stem>/<n>`; the flat form stays
 * readable for sessions snapshotted before the namespacing.
 */
export function sessionTurnRef(turn: number, sessionStem: string, prefix: string = DEFAULT_REF_PREFIX): string {
  // Charset strip then dot-run collapse: git forbids `..` anywhere in a
  // refname, and a stem arrives raw from a session filename.
  const safe = (sessionStem.replace(/[^a-zA-Z0-9._-]/g, "_").replace(/\.{2,}/g, "_")) || "session";
  return `${prefix}/${safe}/${turn}`;
}

/** Engine slot checkpoint filename for one turn (+ session stem). */
export function turnFilename(turn: number, sessionStem = "session"): string {
  const safe = sessionStem.replace(/[^a-zA-Z0-9._-]/g, "_");
  return `${safe}-turn-${turn}.bin`;
}
