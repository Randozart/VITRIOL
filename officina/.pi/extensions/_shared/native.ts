// officina-native loader (2026-09-01) — loads the Rust NAPI addon and
// publishes it on globalThis.__officinaNative so both the TS extensions and
// the patched vendor layout (OfficinaSplit in runtime/patched/) can delegate
// hot paths without Node resolution machinery.
//
// Contract: every consumer keeps a JS fallback. A missing/stale addon is a
// performance loss, never a correctness one. The addon is built from
// officina/native (`npm run native:build`); index.node is a build artifact.
//
// Provenance: original work, this repo (Apache-2.0 OR MIT).

import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export interface OfficinaNative {
  stripAnsi(line: string): string;
  visibleWidth(line: string): number;
  cutLine(line: string, width: number): string;
  mergeSplitRows(input: {
    mainLines: string[];
    mainW: number;
    sbLines: string[];
    sbW: number;
    sbPad: number;
    gap: number;
    bg: string;
    reset: string;
  }): string[];
  renderGauge(ramp: string, ratio: number, cells: number, mutedR: number, mutedG: number, mutedB: number): string;
  rampStops(ramp: string): number[];
  nativeVersion(): string;
}

let cached: OfficinaNative | null = null;
let probed = false;

/** Load the addon once; null when absent/incompatible. Never throws. */
export function getNative(): OfficinaNative | null {
  if (probed) return cached;
  probed = true;
  try {
    const here = dirname(fileURLToPath(import.meta.url));
    // .pi/extensions/_shared → ../../../native/index.node (officina root)
    const p = join(here, "..", "..", "..", "native", "index.node");
    if (!existsSync(p)) return null;
    const req = createRequire(import.meta.url);
    const n = req(p) as OfficinaNative;
    // Sanity: wrong-ABI addon would throw above; wrong-content defends here.
    if (typeof n.visibleWidth !== "function" || typeof n.renderGauge !== "function") return null;
    cached = n;
    (globalThis as any).__officinaNative = n;
  } catch {
    cached = null;
  }
  return cached;
}

/** True when the native addon is active (exported for parity tests/status). */
export function nativeActive(): boolean {
  return getNative() !== null;
}

// Publish the globalThis bridge eagerly: extension modules are imported by
// pi before the interactive TUI constructs OfficinaSplit, so the patched
// vendor code sees the addon on its very first render (no JS-fallback flash).
getNative();
