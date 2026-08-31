// edit-churn — repetition detection for the edit loop. Pure module:
// hashing and counters only, no LLM, no file I/O.
//
// Small models loop: they re-apply the same edit that already failed to
// change anything, burning a full LLM call (with full context) per retry.
// This tracker notices the loop deterministically and returns a directive
// telling the model to change strategy instead.
//
// Provenance: original work, this repo (First-Party Mandate). Plan:
// .opencode/plans/officina-algorithmic-support-2026-08-31.md (P2).

export interface ChurnConfig {
  enabled: boolean;
  /** Repeats of the same old→new content pair before the loop fires. */
  loopThreshold: number;
  /** Total edits to one file before the soft volume warning fires. */
  fileThreshold: number;
}

export function churnConfig(env: NodeJS.ProcessEnv = process.env): ChurnConfig {
  const n = (v: string | undefined, d: number) => {
    const x = v === undefined ? NaN : Number(v);
    return Number.isFinite(x) && x >= 1 ? Math.floor(x) : d;
  };
  return {
    enabled: env.OFFICINA_NO_CHURN !== "1",
    loopThreshold: n(env.OFFICINA_CHURN_LOOP, 3),
    fileThreshold: n(env.OFFICINA_CHURN_FILE, 10),
  };
}

export interface EditObservation {
  file: string;
  oldHash: string;
  newHash: string;
}

/** A churn directive for the model, or null when the edit looks healthy. */
export interface ChurnDirective {
  message: string;
}

export class ChurnTracker {
  private pairs = new Map<string, number>();
  private perFile = new Map<string, number>();
  private volumeWarned = new Set<string>();

  constructor(private cfg: ChurnConfig) {}

  /** Record one completed edit; returns a directive when churn is detected. */
  record(obs: EditObservation): ChurnDirective | null {
    const edits = (this.perFile.get(obs.file) ?? 0) + 1;
    this.perFile.set(obs.file, edits);

    const key = `${obs.oldHash}>${obs.newHash}`;
    const seen = (this.pairs.get(key) ?? 0) + 1;
    this.pairs.set(key, seen);

    if (seen >= this.cfg.loopThreshold) {
      return {
        message:
          `edit churn: this exact edit has now been applied to ${obs.file} ${seen} times ` +
          "without changing the outcome — it is NOT sticking. Re-read the file fresh " +
          "and change strategy (different anchor text, a rewrite, or a different fix).",
      };
    }
    if (edits >= this.cfg.fileThreshold && !this.volumeWarned.has(obs.file)) {
      this.volumeWarned.add(obs.file);
      return {
        message:
          `edit churn: ${edits} edits to ${obs.file} this session. If you are iterating ` +
          "toward a fix, consider rewriting the whole block once instead of piecemeal edits.",
      };
    }
    return null;
  }
}
