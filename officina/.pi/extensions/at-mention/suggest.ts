// PROVENANCE: original — VITRIOL officina (2026-09-27, parity plan phase 1.2).
// Pure suggestion logic for at-mention: @-token extraction, subsequence fuzzy
// scoring, and merge into the stock provider's suggestion set. No I/O here —
// the caller owns the catalog and line access. Unit-tested in suggest.test.ts.

export interface CatalogEntry {
  /** stable identity (dedupe across refreshes) */
  id: string;
  /** text inserted into the editor on apply */
  value: string;
  /** display name (directories conventionally end with "/") */
  label: string;
  description?: string;
  dir: boolean;
}

export interface AtToken {
  /** matched token including the leading "@" (the editor's prefix) */
  raw: string;
  /** query without "@" ("" for bare @) */
  query: string;
}

/** The @-token ending exactly at `col` on `line`, or null. Mirrors the stock
 *  provider's token rule: "@" at a token boundary (start or after whitespace),
 *  no whitespace between "@" and the cursor. */
export function atTokenAt(line: string, col: number): AtToken | null {
  const upTo = line.slice(0, col);
  const m = /(?:^|\s)@([^\s]*)$/.exec(upTo);
  if (!m) return null;
  return { raw: m[0].slice(m[0].indexOf("@") === 0 ? 0 : 1), query: m[1] ?? "" };
}

/** Subsequence match score in [0,1]; 0 = no match. Case-insensitive.
 *  Contiguous runs and boundary hits score higher (fuzzy-insertion style). */
export function fuzzyScore(haystack: string, needle: string): number {
  if (needle.length === 0) return 1;
  const h = haystack.toLowerCase();
  const n = needle.toLowerCase();
  let hi = 0;
  let score = 0;
  let run = 0;
  for (let ni = 0; ni < n.length; ni++) {
    const idx = h.indexOf(n[ni], hi);
    if (idx === -1) return 0;
    // bonus: match right after previous (contiguous run) or at a boundary
    if (idx === hi && ni > 0) {
      run += 1;
      score += 2 + run;
    } else {
      run = 0;
      score += idx === 0 || /[\s/._-]/.test(h[idx - 1] ?? "") ? 2 : 1;
    }
    hi = idx + 1;
  }
  // prefer tighter haysticks
  return score / (h.length + n.length);
}

/** Score one entry against the query; label weighted above description/path. */
export function scoreEntry(entry: CatalogEntry, query: string): number {
  const label = fuzzyScore(entry.label, query);
  if (label <= 0) return 0;
  const pathBoost = fuzzyScore(entry.value, query) * 0.2;
  const kindBoost = entry.description ? fuzzyScore(entry.description, query) * 0.15 : 0;
  return label * 2 + pathBoost + kindBoost;
}

type AtTokenFn = (line: string, col: number) => AtToken | null;

interface BaseSuggestions {
  items: { value: string; label: string; description?: string }[];
  prefix: string;
}

/** Merge officina entries into the stock provider's result. Extras carry a
 *  marker so index.ts's applyCompletion can own their insertion. Never
 *  duplicates a value already suggested by the base; caps extras at 6 so the
 *  stock file hits keep priority. Returns null when there is no @-token and
 *  the base has nothing. */
export function mergeSuggestions(
  base: BaseSuggestions | null,
  entries: CatalogEntry[],
  lines: string[],
  cursorLine: number,
  cursorCol: number,
  atToken: AtTokenFn,
  cap = 6,
): { items: (BaseSuggestions["items"][number] & { officina?: boolean })[]; prefix: string } | null {
  const token = atToken(lines[cursorLine] ?? "", cursorCol);
  if (!token) return base;
  const scored = entries
    .map((e) => ({ e, s: scoreEntry(e, token.query) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s)
    .slice(0, cap)
    .map(({ e }) => ({
      value: e.value,
      label: e.label + (e.dir ? "/" : ""),
      description: e.description,
      officina: true as const,
    }));
  const baseItems = base?.items ?? [];
  const seen = new Set(baseItems.map((i) => i.value));
  const items = [...baseItems, ...scored.filter((s) => !seen.has(s.value))];
  if (items.length === 0) return null;
  return { items, prefix: token.raw };
}
