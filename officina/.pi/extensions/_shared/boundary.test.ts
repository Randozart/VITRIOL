// Extension boundary (2026-11-18).
//
// An extension may import `_shared/*` and its own directory. Importing ANOTHER
// extension is the coupling that makes Officina spaghetti: it ties two
// separately-loaded jiti instances together, so neither can be changed or
// switched off alone (e.g. `phase-model` reaching into
// `llama-cpp-provider/config.ts`, or `_shared` importing a feature module —
// a genuine inversion).
//
// The list below is FROZEN DEBT: it records what exists today so the boundary
// can only shrink. Adding an entry is a review decision, not a convenience —
// the fix is almost always to move the shared piece into `_shared/`.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const extensionsDir = resolve(here, "..");

/** Known cross-extension imports, 2026-11-18. May only shrink. */
const FROZEN: string[] = [
  "_shared -> vitriol-decode",
  "adaptive-router -> auto-route",
  "adaptive-router -> caduceus",
  "code-tool -> diagnostics-loop",
  "code-tool -> format-gate",
  "code-tool -> praetor",
  "code-tool -> verify",
  "composer-predictions -> auto-route",
  "composer-predictions -> caduceus",
  "composer-predictions -> hang-watchdog",
  "composer-predictions -> task-state",
  "deep-research -> plan-mode",
  "deep-research -> subagent",
  "fix-loop -> verify",
  "grind -> task-state",
  "grind -> verify",
  "hang-watchdog -> caduceus",
  "knowledge-inject -> skill-inject",
  "permissions-guard -> longjob",
  "phase-model -> llama-cpp-provider",
  "plan-mode -> phase-model",
  "plan-mode -> subagent",
  "resilient-default -> adaptive-router",
  "resilient-default -> auto-route",
  "resilient-default -> caduceus",
  "scratchpad -> task-state",
  "session-panel -> knowledge-inject",
  "session-panel -> scratchpad",
  "session-panel -> skill-inject",
  "session-panel -> task-state",
  "session-panel -> vitriol-decode",
  "verify-on-edit -> verify",
];

function discover(): string[] {
  const pairs = new Set<string>();
  const walk = (dir: string): void => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
        walk(full);
        continue;
      }
      if (!entry.name.endsWith(".ts") || entry.name.endsWith(".test.ts")) continue;
      const rel = relative(extensionsDir, full);
      const owner = rel.split("/")[0];
      const text = readFileSync(full, "utf8");
      for (const match of text.matchAll(/from "\.\.\/([a-z0-9_-]+)\//g)) {
        const target = match[1];
        if (target === "_shared" || target === owner) continue;
        pairs.add(`${owner} -> ${target}`);
      }
    }
  };
  walk(extensionsDir);
  return [...pairs].sort();
}

describe("extension boundary", () => {
  it("has no NEW cross-extension imports", () => {
    const fresh = discover().filter((pair) => !FROZEN.includes(pair));
    expect(fresh).toEqual([]);
  });

  it("keeps the frozen list honest (no stale entries)", () => {
    // A migrated pair must be removed from FROZEN, so the ratchet cannot
    // silently re-open a hole that was already fixed.
    const found = new Set(discover());
    expect(FROZEN.filter((pair) => !found.has(pair))).toEqual([]);
  });

  it("keeps the Rust TUI free of the legacy JS frontend", () => {
    // officina.mjs + runtime/patched/* are DEPRECATED (AGENTS.md 2026-11-10) but
    // still on disk because scripts/selfcheck.sh drives a one-shot round trip
    // through them. Deleting them is a separate task (rewrite that check against
    // `pi -p`); until then this gate makes sure the Rust TUI never grows a
    // dependency on them, so the deletion stays a pure subtraction.
    const tuiSrc = resolve(extensionsDir, "..", "..", "tui", "src");
    const offenders: string[] = [];
    const walk = (dir: string): void => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name);
        if (entry.isDirectory()) {
          walk(full);
          continue;
        }
        if (!entry.name.endsWith(".rs")) continue;
        // Comments may reference the legacy entry point (parity notes); a
        // DEPENDENCY is code, so only non-comment lines count.
        const code = readFileSync(full, "utf8")
          .split("\n")
          .filter((line) => !/^\s*(\/\/|\/\/!)|^\s*#/.test(line))
          .join("\n");
        if (/runtime\/patched|officina\.mjs|register-hooks/.test(code)) {
          offenders.push(entry.name);
        }
      }
    };
    walk(tuiSrc);
    expect(offenders).toEqual([]);
  });

  it("keeps support code from importing features (inversion)", () => {
    expect(discover().filter((p) => p.startsWith("_shared -> "))).toEqual([
      "_shared -> vitriol-decode",
    ]);
  });
});
