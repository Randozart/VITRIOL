// Below-composer band registry (2026-11-18).
//
// The strip between the transcript and the prompt had THREE rows and SIXTEEN
// writers, ordered by a hardcoded `match` inside the Rust layout that named five
// widget keys and dropped everything else into `_ => 9`. That is how a
// composer *suggestion* ("▸ next: …") ended up rendering directly under the
// routing *state* row ("route manual — overflow paused").
//
// This module makes the strip a contract:
//
//   tier "state"       what the harness is doing right now (route, decode,
//                       plan/deep-research/phase). Always wins a row.
//   tier "progress"    live counters that accompany state.
//   tier "suggestion"  advisory text (the next step). NEVER renders while any
//                       state/progress row is present — a suggestion is not
//                       status, and it must never push status off the strip.
//
// One module assembles and writes ONE widget (`officina-band`); producers call
// `setBandRow`/`clearBandRow` and never touch `ui.setWidget` for the band
// themselves. Flush is deduped, so a 10 Hz producer cannot churn the widget
// container (which disturbs scrolling).
//
// globalThis-backed (2026-09-06 jiti isolation rule): pi loads every extension
// in its own jiti instance, so a module-level Map would give each producer a
// private registry and nobody would ever see anybody's rows.

export type BandTier = "state" | "progress" | "suggestion";

/** Widget key the TUI renders first (layout.rs: `officina-band` => 0). */
export const BAND_WIDGET_KEY = "officina-band";

/** Row budget for the strip (matches the TUI's settled band height). */
export const BAND_MAX_ROWS = 3;

interface BandRow {
  tier: BandTier;
  lines: string[];
}

interface BandState {
  rows: Map<string, BandRow>;
  /** Last ui handle seen, so any producer can trigger a flush. */
  ui?: BandUi;
  /** Last flushed payload: identical flushes are skipped. */
  last?: string;
}

interface BandUi {
  setWidget?: (
    key: string,
    lines: string[],
    options?: { placement?: string },
  ) => void;
}

const state: BandState =
  (globalThis as unknown as { __officinaBand?: BandState }).__officinaBand ??
  ((globalThis as unknown as { __officinaBand?: BandState }).__officinaBand = {
    rows: new Map<string, BandRow>(),
  });

const TIER_ORDER: Record<BandTier, number> = {
  state: 0,
  progress: 1,
  suggestion: 2,
};

/**
 * Within the `state` tier the operator's scan order matters: the route strip is
 * the primary target, then live decode, then the mode indicators. This list is
 * THE band priority table — it used to be a `match` in the Rust layout naming
 * five widget keys, which is how an unknown key (the composer prediction) fell
 * into the strip at priority 9. Unknown ids sort after the known ones, stable by
 * insertion, so a new producer still renders.
 */
const STATE_ORDER = [
  "overflow-route",
  "vitriol-decode",
  "plan-mode",
  "deep-research-mode",
  "phase-model",
];

function rank(id: string, row: BandRow): number {
  const tier = TIER_ORDER[row.tier];
  if (row.tier !== "state") return tier * 100;
  const at = STATE_ORDER.indexOf(id);
  return tier * 100 + (at === -1 ? STATE_ORDER.length : at);
}

function normalize(lines: string[] | string | null | undefined): string[] {
  if (lines === null || lines === undefined) return [];
  const list = Array.isArray(lines) ? lines : [lines];
  return list.map((l) => (l ?? "").trimEnd()).filter((l) => l.trim() !== "");
}

/**
 * Publish (or replace) one row. `lines` empty/null clears the row.
 * Producers call this whenever their content changes; flushing is automatic.
 *
 * Rows are keyed by PRODUCER ID, not by (id, tier): a producer owns exactly one
 * row and declares its tier each time it publishes. Consequently
 * `setBandRow(id, tierA, null)` also removes the row it published as tier B —
 * clear-then-publish, never publish-then-clear (2026-11-19, found by the decode
 * flap test: clearing the state tier silently deleted the progress row).
 */
export function setBandRow(
  id: string,
  tier: BandTier,
  lines: string[] | string | null | undefined,
): void {
  const next = normalize(lines);
  if (next.length === 0) {
    clearBandRow(id);
    return;
  }
  state.rows.set(id, { tier, lines: next });
  flushBand();
}

/** Remove a row (the producing extension went quiet or was disabled). */
export function clearBandRow(id: string): void {
  if (!state.rows.delete(id)) return;
  flushBand();
}

/** Remember the ui handle so later producers can trigger a flush themselves. */
export function setBandUi(ui: BandUi | undefined): void {
  if (!ui) return;
  state.ui = ui;
}

/**
 * Request a flush (after a transient state change that does not alter rows).
 * Safe to call from any extension instance.
 */
export function requestBandUpdate(): void {
  flushBand();
}

/**
 * Pure assembly: rows in tier order, suggestion rows only when nothing else is
 * showing, capped at BAND_MAX_ROWS. Exported for tests — the rule that matters
 * is here, not in the renderer.
 */
export function bandLines(
  rows: Iterable<[string, BandRow]> = state.rows,
  limit: number = BAND_MAX_ROWS,
): string[] {
  const all = [...rows];
  const status = all.filter(([, row]) => row.tier !== "suggestion");
  // A suggestion is not status: when anything is reporting state, suggestions
  // stay out of the strip entirely (the 2026-11-18 "▸ next" under "route" bug).
  const visible = status.length > 0 ? status : all;
  return visible
    .map((entry, i) => ({ entry, i, rank: rank(entry[0], entry[1]) }))
    .sort((a, b) => a.rank - b.rank || a.i - b.i)
    .flatMap(({ entry }) => entry[1].lines)
    .slice(0, limit);
}

/** Write the assembled strip. No-op when the payload is unchanged. */
export function flushBand(): void {
  const ui = state.ui;
  if (!ui?.setWidget) return;
  const lines = bandLines();
  const payload = JSON.stringify(lines);
  if (payload === state.last) return;
  state.last = payload;
  try {
    ui.setWidget(BAND_WIDGET_KEY, lines, { placement: "belowEditor" });
  } catch {
    // decoration contract: never break the workshop
  }
}

/** Test hook: wipe rows and the cached payload. */
export function resetBand(): void {
  state.rows.clear();
  state.last = undefined;
}

/** Everything a test needs to inspect the registry. */
export function bandSnapshot(): {
  id: string;
  tier: BandTier;
  lines: string[];
}[] {
  return [...state.rows].map(([id, row]) => ({
    id,
    tier: row.tier,
    lines: row.lines,
  }));
}
