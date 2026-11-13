// Command-name uniqueness guard (2026-11-13).
//
// The /cost collision happened because the TUI's LOCAL_COMMANDS silently
// shadowed an extension command with the same name. This test scans both
// registries and fails on ANY duplicate — extension↔extension,
// local↔local, or extension↔local.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = join(__dirname, "..");
const EXTENSIONS = join(ROOT, ".pi", "extensions");
const TUI_STATE = join(ROOT, "tui", "src", "tui", "state.rs");

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...walk(p));
    else if (name.endsWith(".ts") && !name.endsWith(".test.ts")) out.push(p);
  }
  return out;
}

/** Extension command names from every registerCommand("…") call site. */
export function extensionCommandNames(root = EXTENSIONS): string[] {
  const names: string[] = [];
  for (const file of walk(root)) {
    const src = readFileSync(file, "utf8");
    for (const m of src.matchAll(/registerCommand\(\s*"([^"]+)"/g)) {
      names.push(m[1]);
    }
  }
  return names.sort();
}

/** TUI-local command names from the LOCAL_COMMANDS table. */
export function localCommandNames(src = readFileSync(TUI_STATE, "utf8")): string[] {
  const block = /pub const LOCAL_COMMANDS[^=]*=\s*&\[([\s\S]*?)\n    \];/.exec(src);
  const body = block?.[1] ?? "";
  return [...body.matchAll(/^\s*\("([a-z0-9-]+)",/gm)].map((m) => m[1]).sort();
}

describe("command-name registry", () => {
  it("has no duplicate extension commands", () => {
    const all = extensionCommandNames();
    const dups = all.filter((n, i) => all.indexOf(n) !== i);
    expect(dups, `duplicate extension commands: ${[...new Set(dups)]}`).toEqual([]);
  });

  it("has no duplicate TUI-local commands", () => {
    const all = localCommandNames();
    const dups = all.filter((n, i) => all.indexOf(n) !== i);
    expect(dups, `duplicate local commands: ${[...new Set(dups)]}`).toEqual([]);
  });

  it("has no local command shadowing an extension command (the /cost bug)", () => {
    const local = new Set(localCommandNames());
    const shadowed = extensionCommandNames().filter((n) => local.has(n));
    expect(shadowed, `TUI shadows extension commands: ${shadowed}`).toEqual([]);
  });

  it("renamed commands are gone and their replacements exist", () => {
    const ext = new Set(extensionCommandNames());
    const local = new Set(localCommandNames());
    const all = new Set([...ext, ...local]);
    // dead names must not exist anywhere
    for (const dead of [
      "route-catalog",
      "overflow-catalog",
      "session-name",
      "panel",
      "fire",
      "glimmer",
      "profile",
      "coupling",
      "model-handover",
      "phase-models",
    ]) {
      expect(all.has(dead), `dead command still registered: ${dead}`).toBe(false);
    }
    // replacements must exist
    for (const live of [
      "route-why",
      "route-overflow",
      "model-profile",
      "model-swap",
      "model-phase",
      "model-phases",
      "provider-health",
      "provider-keys",
      "provider-add",
      "budget",
      "import-tasks",
      "import-opencode",
    ]) {
      expect(all.has(live), `replacement command missing: ${live}`).toBe(true);
    }
  });
});
