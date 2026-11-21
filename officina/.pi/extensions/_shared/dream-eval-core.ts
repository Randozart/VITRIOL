// dream-eval-core — the frozen gate every dream-pipeline stage stands on
// (2026-11-21, plan dream-pipeline-2026-11-21.md).
//
// Purpose: answer "is model/adapter X at least as good as the baseline on
// OUR domain?" with a measurement that cannot drift. The eval set is frozen
// by content hash (I-E1): every report embeds the hash; a changed hash means
// a different eval and a reset baseline. Train/eval separation is structural
// (I-E2): rows carry their split, and the split assignment is part of the
// frozen content.
//
// PURE: no fs, no HTTP, no timers. The runner (dream-eval extension) resolves
// endpoints and feeds answers in.
//
// Ground truth (I-P3): every eval item carries provenance — the ledger row
// or MEMORY fact it was derived from. Untraceable items are dropped at build
// time (buildEvalSet drops provenance-less entries).

/** Question classes with protection levels (I-P1: a promotion must not
 *  regress a protected class even if overall pass-rate rises). */
export type QuestionClass =
  | "command" // exact-match: shell/git/engine invocations
  | "number" // numeric facts: memory budgets, tensor splits, versions
  | "convention"; // project conventions: judged by content overlap

/** One eval item: a question with its known-good answer requirements. */
export interface EvalItem {
  id: string;
  question: string;
  /** Substrings that MUST appear in a correct answer (exact for commands/
   *  numbers, case-insensitive for conventions). */
  mustContain: string[];
  class: QuestionClass;
  /** Provenance: ledger row id or MEMORY fact line (I-P3). */
  provenance: string;
}

export interface EvalSet {
  /** sha256 over the canonical JSON of the items — I-E1. */
  hash: string;
  createdAt: string;
  items: Array<EvalItem & { split: "train" | "eval" }>;
}

// ── hashing ───────────────────────────────────────────────────────────────

/** sha256 hex of a string (node crypto — available in the extension host). */
export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return [...new Uint8Array(digest)]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

/** Canonical JSON: sorted keys, no whitespace — hashing is order-stable. */
export function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(",")}]`;
  }
  const keys = Object.keys(value as Record<string, unknown>).sort();
  return `{${keys
    .map(
      (k) =>
        `${JSON.stringify(k)}:${canonicalJson((value as Record<string, unknown>)[k])}`,
    )
    .join(",")}}`;
}

/**
 * Build a frozen eval set from candidate items.
 * - Drops items without provenance (I-P3).
 * - Assigns splits deterministically: eval = every Nth item (default 10),
   counted per class so each class keeps eval coverage (I-E2).
 * - Hash = sha256 over the canonical JSON of the FULL item list (both
   splits): a changed item anywhere changes the hash (I-E1).
 */
export async function buildEvalSet(
  candidates: Array<EvalItem>,
  opts: { evalEveryN?: number; createdAt?: string } = {},
): Promise<EvalSet> {
  const traced = candidates.filter(
    (c) => typeof c.provenance === "string" && c.provenance.length > 0,
  );
  const evalEveryN = Math.max(2, opts.evalEveryN ?? 10);
  const byClass = new Map<QuestionClass, number>();
  const items = traced.map((c) => {
    const seen = byClass.get(c.class) ?? 0;
    byClass.set(c.class, seen + 1);
    const split: "eval" | "train" =
      seen % evalEveryN === 0 ? "eval" : "train";
    return { ...c, split };
  });
  const hash = await sha256Hex(canonicalJson(items));
  return {
    hash,
    createdAt: opts.createdAt ?? new Date().toISOString(),
    items,
  };
}

// ── judging ───────────────────────────────────────────────────────────────

export interface JudgedItem {
  id: string;
  class: QuestionClass;
  passed: boolean;
  /** Why it failed ("" when passed). */
  reason: string;
}

/**
 * Judge one answer against its item.
 *  - command class: EVERY mustContain string appears verbatim (case-sensitively)
 *    — commands are identity.
 *  - number class: same, verbatim (numbers are identity).
 *  - convention class: case-insensitive contains, ≥60% of mustContain strings.
 */
export function judge(item: EvalItem, answer: string): JudgedItem {
  const ans = answer ?? "";
  if (item.class === "command" || item.class === "number") {
    const missing = item.mustContain.filter((s) => !ans.includes(s));
    return {
      id: item.id,
      class: item.class,
      passed: missing.length === 0,
      reason: missing.length ? `missing: ${missing.join(" | ")}` : "",
    };
  }
  const lower = ans.toLowerCase();
  const hit = item.mustContain.filter((s) =>
    lower.includes(s.toLowerCase()),
  ).length;
  const ratio = item.mustContain.length ? hit / item.mustContain.length : 1;
  return {
    id: item.id,
    class: item.class,
    passed: ratio >= 0.6,
    reason: ratio >= 0.6 ? "" : `coverage ${(ratio * 100).toFixed(0)}% < 60%`,
  };
}

// ── scoring + promotion ───────────────────────────────────────────────────

export interface EvalReport {
  hash: string;
  model: string;
  lora: string | null;
  /** Overall pass rate on the eval split. */
  passRate: number;
  /** Pass rate per class. */
  perClass: Record<QuestionClass, { passed: number; total: number }>;
  /** Class-level regressions vs baseline (I-P1: any entry blocks promotion). */
  regressions: QuestionClass[];
  judged: JudgedItem[];
}

/** Score a set of judged items (eval split only). */
export function score(
  items: Array<EvalItem & { split: "train" | "eval" }>,
  judged: Map<string, JudgedItem>,
  hash: string,
  model: string,
  lora: string | null = null,
): EvalReport {
  const evalItems = items.filter((i) => i.split === "eval");
  const perClass = {} as EvalReport["perClass"];
  let passed = 0;
  for (const item of evalItems) {
    const j = judged.get(item.id);
    const c = (perClass[item.class] ??= { passed: 0, total: 0 });
    c.total++;
    if (j?.passed) {
      c.passed++;
      passed++;
    }
  }
  return {
    hash,
    model,
    lora,
    passRate: evalItems.length ? passed / evalItems.length : 0,
    perClass,
    regressions: [],
    judged: evalItems
      .map((i) => judged.get(i.id))
      .filter(Boolean) as JudgedItem[],
  };
}

/**
 * Promotion rule (I-P1): candidate pass-rate ≥ baseline AND no per-class
 * regression below the baseline's class pass-rate.
 */
export function promotionDecision(
  baseline: EvalReport,
  candidate: EvalReport,
): { promote: boolean; reasons: string[] } {
  const reasons: string[] = [];
  if (candidate.hash !== baseline.hash) {
    reasons.push(
      `eval hash mismatch: ${candidate.hash} vs baseline ${baseline.hash} — different eval, reset the baseline`,
    );
  }
  if (candidate.passRate < baseline.passRate) {
    reasons.push(
      `overall ${(candidate.passRate * 100).toFixed(1)}% < baseline ${(baseline.passRate * 100).toFixed(1)}%`,
    );
  }
  for (const cls of Object.keys(baseline.perClass) as QuestionClass[]) {
    const b = baseline.perClass[cls];
    const c = candidate.perClass[cls];
    if (!b || !c) continue;
    const bRate = b.total ? b.passed / b.total : 1;
    const cRate = c.total ? c.passed / c.total : 1;
    if (cRate < bRate) {
      reasons.push(
        `${cls} regressed: ${(cRate * 100).toFixed(0)}% < baseline ${(bRate * 100).toFixed(0)}%`,
      );
    }
  }
  return { promote: reasons.length === 0, reasons };
}
