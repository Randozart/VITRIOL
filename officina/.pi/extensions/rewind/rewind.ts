import { turnFilename } from "../_shared/turnkeys.ts";

export { turnFilename };

// rewind — pair-restore UX (Round 4 gap D2-④): /rewind <turn> restores the
// WORKTREE from the snapshot ref AND the engine KV from the checkpoint file,
// both keyed by the shared convention (snapshot: refs/trismegistus/turns/N,
// checkpoint: <session>-turn-N.bin). Convention, not import-coupling: the
// halves may exist without each other and degrade honestly.

export interface TurnPair {
  turn: number;
  code: boolean; // snapshot ref exists
  kv: "attempt"; // KV restored optimistically — only the engine knows
}

/** Turn numbers from `git for-each-ref --format=%(refname) refs/.../turns`. */
export function parseRefs(out: string, prefix = "refs/trismegistus/turns/"): number[] {
  const turns: number[] = [];
  for (const line of out.split("\n")) {
    const t = line.trim();
    if (!t.startsWith(prefix)) continue;
    const n = Number(t.slice(prefix.length).split(/\s+/)[0]);
    if (Number.isInteger(n) && n >= 0) turns.push(n);
  }
  return turns.sort((a, b) => b - a);
}

export function pairTurns(turns: number[], current: number): TurnPair[] {
  return [...turns]
    .sort((a, b) => b - a)
    .filter((t) => t <= current)
    .slice(0, 10)
    .map((t) => ({ turn: t, code: true, kv: "attempt" as const }));
}

/** Human plan shown in the confirm dialog. */
export function formatPlan(turn: number, filename: string, pairs: TurnPair[]): string {
  const list = pairs.map((p) => p.turn).join(", ");
  return (
    `REWIND to turn ${turn}\n` +
    `  code: worktree <- refs/trismegistus/turns/${turn} (files land in worktree+index)\n` +
    `  KV:   engine slot <- ${filename} (attempt; fails soft if never saved)\n` +
    `  snapshots available: ${list || "none"}\n` +
    `THIS OVERWRITES UNCOMMITTED WORK from later turns.`
  );
}

