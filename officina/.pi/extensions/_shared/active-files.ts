// active-files — shared tracker for files actively being edited or read.
//
// Used by tool-result-clearer to protect tool results for active files from
// eviction. When the model is editing main.rs, a stale read of main.rs is
// NOT safe to clear — the model still needs the content to reason about its
// edits.
//
// Module-level singleton: one per process, shared across all extensions.
// TTL is turn-based (not wall-clock) because the model's reasoning loop is
// turn-based. A file touched 3 turns ago is still hot; one untouched for 20
// turns is cold.
//
// Kill switch: OFFICINA_NO_ACTIVE_FILES=1 (Rule 15).
// TTL: OFFICINA_ACTIVE_TTL (default 10 turns).

import { readFileSync } from "node:fs";

// globalThis-backed (2026-11-18): the comment that used to sit here claimed "one
// per process, shared across all extensions". That was FALSE — pi loads every
// extension through its own jiti instance (moduleCache: false), so each got a
// private Map and the tool-result-clearer's "do not clear a hot file" protection
// silently missed files another extension had registered. Same rule as
// `_shared/sidebar.ts` and the routing registry.
interface ActiveFilesState {
  byFile: Map<string, number>;
  turn: number;
}
const shared: ActiveFilesState =
  (globalThis as unknown as { __officinaActiveFiles?: ActiveFilesState })
    .__officinaActiveFiles ??
  ((globalThis as unknown as { __officinaActiveFiles?: ActiveFilesState })
    .__officinaActiveFiles = { byFile: new Map<string, number>(), turn: 0 });

/** Test hook: forget every registration and reset the logical clock. */
export function resetActiveFiles(): void {
  shared.byFile.clear();
  shared.turn = 0;
}

const activeByFile = shared.byFile; // file → lastActiveTurn


export function activeFilesEnabled(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.OFFICINA_NO_ACTIVE_FILES !== "1";
}

/** Runtime enable flag (env kill switch AND the test/runtime toggle). */
function enabledNow(): boolean {
  if (!activeFilesEnabled()) return false;
  return !(globalThis as unknown as { __officinaActiveFilesOff?: boolean })
    .__officinaActiveFilesOff;
}

export function getActiveTTL(env: NodeJS.ProcessEnv = process.env): number {
  const raw = env.OFFICINA_ACTIVE_TTL;
  if (raw === undefined) return 10;
  const n = Number(raw);
  return Number.isFinite(n) && n >= 1 ? Math.floor(n) : 10;
}

/** Advance the logical clock. Called once per context pass. */
export function tickTurn(): void {
  shared.turn++;
}

/** Register a file as actively touched at the current turn. */
export function register(file: string, turn?: number): void {
  if (!file || !enabledNow()) return;
  activeByFile.set(file, turn ?? shared.turn);
}

/** True if the file was touched within the last `ttl` turns. */
export function isActive(file: string, ttl?: number): boolean {
  if (!file || !enabledNow()) return false;
  const limit = ttl ?? getActiveTTL();
  const last = activeByFile.get(file);
  if (last === undefined) return false;
  // Evict stale entries on access
  if (shared.turn - last > limit) {
    activeByFile.delete(file);
    return false;
  }
  return true;
}

/** Return all currently active file paths. */
export function getActive(): Set<string> {
  const ttl = getActiveTTL();
  const out = new Set<string>();
  for (const [file, last] of activeByFile) {
    if (shared.turn - last <= ttl) {
      out.add(file);
    } else {
      activeByFile.delete(file);
    }
  }
  return out;
}

/**
 * Read the current task file and register file paths mentioned in in_progress
 * task descriptions. Heuristic extraction: paths ending in common source
 * extensions (.rs, .ts, .js, .py, .md, .json, .toml).
 */
export function registerTaskFiles(taskFilePath: string): void {
  if (!enabledNow()) return;
  try {
    const data = JSON.parse(readFileSync(taskFilePath, "utf8")) as {
      tasks?: Array<{ status?: string; description?: string }>;
    };
    for (const t of data.tasks ?? []) {
      if (t.status !== "in_progress" || !t.description) continue;
      const matches = t.description.match(
        /[\w/.-]+\.(rs|ts|tsx|js|jsx|py|md|json|toml|yaml|yml|mlir|c|h|cpp)/g,
      );
      if (matches) {
        for (const m of matches) register(m);
      }
    }
  } catch {
    // missing or corrupt task file — no protection, not an error
  }
}

/** Reset all state (for testing). */
export function _reset(): void {
  activeByFile.clear();
  shared.turn = 0;
}

/**
 * Enable/disable tracking. The flag is globalThis-backed too, so a kill switch
 * set by one extension instance is seen by the others (jiti isolation).
 */
export function _setEnabled(v: boolean): void {
  (globalThis as unknown as { __officinaActiveFilesOff?: boolean }).__officinaActiveFilesOff =
    !v;
}
