// Shared-state inventory gate (2026-11-18).
//
// Several Officina sessions share ~/.vitriol/officina, and every file there is a
// potential cross-session channel. This test discovers the paths the sources
// actually use and diffs them against `SHARED_STATE`, so a new shared file fails
// CI until someone classifies it (session-keyed / last-writer-wins /
// append-only / user-config).

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { SHARED_STATE, collides, unclassified } from "./shared-state.ts";

const here = dirname(fileURLToPath(import.meta.url));
const extensionsDir = resolve(here, "..");

/** Every non-test source file under .pi/extensions. */
function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "node_modules" || entry.name.startsWith(".")) continue;
      sources(path, out);
      continue;
    }
    if (!entry.name.endsWith(".ts") || entry.name.endsWith(".test.ts"))
      continue;
    out.push(path);
  }
  return out;
}

/**
 * Names used as `".vitriol", "officina", "<name>"` — the shape every writer
 * takes. Kept deliberately narrow: a wider scan (any path fragment) would flag
 * comments and test fixtures, and a gate that cries wolf gets disabled.
 */
const NAME_RE = /"\.vitriol",\s*"officina",\s*"([a-zA-Z0-9._-]+)"/g;

function discover(): { names: string[]; files: number } {
  const names: string[] = [];
  const files = sources(extensionsDir);
  for (const file of files) {
    if (!statSync(file).isFile()) continue;
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(NAME_RE)) names.push(match[1]);
  }
  return { names, files: files.length };
}

describe("shared state inventory", () => {
  it("classifies every shared path the sources use", () => {
    const { names, files } = discover();
    // Sanity: the scan must actually see the tree, or "nothing unclassified"
    // would be trivially true.
    expect(files).toBeGreaterThan(50);
    expect(names.length).toBeGreaterThan(10);
    expect(unclassified(names)).toEqual([]);
  });

  it("reports an unclassified name (negative control)", () => {
    // The gate is only worth having if a new file actually fails it.
    expect(unclassified(["brand-new-state.json"])).toEqual([
      "brand-new-state.json",
    ]);
    expect(unclassified([...Object.keys(SHARED_STATE)])).toEqual([]);
  });

  it("keeps the known collision surface explicit and small", () => {
    // These are the files where two sessions genuinely overwrite each other.
    // Pinned so growing the list is a decision, not a drift.
    const discovered = discover().names;
    expect(collides(discovered)).toEqual([
      "agent-mode.json",
      "caduceus-models.json",
      "caduceus-state.json",
      "gateway-health.json",
      "inquire-answer.json",
      "provider-health.json",
      "question-response.json",
      "question.json",
      "routing-registry.json",
    ]);
  });

  it("keys the paths that must never collide", () => {
    expect(SHARED_STATE["routing-registry.d"].kind).toBe("session-keyed");
    expect(SHARED_STATE["compact-on-finish"].kind).toBe("session-keyed");
    expect(SHARED_STATE["route-ledger-"].kind).toBe("append-only");
  });
});
