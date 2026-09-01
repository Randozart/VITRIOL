// agent-mode shared registry — mode definitions + current-mode state, so
// every surface (agent-mode widget, session-panel badge + frame) renders
// from one source.
//
// Modes are CONFIGURABLE (owner request 2026-09-01): defaults are built in,
// and a JSON file (~/.vitriol/officina/modes.json, or OFFICINA_MODES=path)
// can override existing modes or add new ones by name. Schema per mode:
//
//   {
//     "name": "plan",                  // unique id (used by /mode <name>)
//     "label": "PLAN",                 // badge text
//     "glyph": "►",                    // badge glyph
//     "color": "sovereignty",          // Vitriolum name OR #rrggbb
//     "hint": "research only, *.md writes",
//     "directive": "AGENT MODE: ...",  // injected EVERY turn in this mode
//     "enterDirective": "...",         // injected ONCE after switching in
//     "blockNonMdWrites": true         // tool gate: block non-.md writes
//   }
//
// Provenance: original work, this repo (First-Party Mandate).

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { VITRIOLUM, hexToRgb } from "./vitriolum.ts";

export interface ModeDef {
  name: string;
  label: string;
  glyph: string;
  color: string; // Vitriolum palette name or #rrggbb
  hint: string;
  directive?: string;
  enterDirective?: string;
  blockNonMdWrites?: boolean;
}

export const DEFAULT_MODES: ModeDef[] = [
  {
    name: "build",
    label: "BUILD",
    glyph: "▪",
    color: "coldBlue", // bold blue: heads-down work
    hint: "writes unlocked · TAB / /mode plan for research",
    enterDirective: `AGENT MODE: BUILD.
Plan mode has ended — you are allowed to modify files and run state-changing commands again, effective immediately. Apply the plan (or the current request) with normal write/edit tools.`,
  },
  {
    name: "plan",
    label: "PLAN",
    glyph: "►",
    color: "sovereignty", // gold: research is the treasure
    hint: "research only, *.md writes",
    directive: `AGENT MODE: PLAN (research-first).
Behavior for this mode: investigate before acting — read the relevant code, trace the call paths, check tests and docs. Think about approaches and trade-offs. You may ONLY create or modify Markdown files (*.md) — use them for notes, findings, and the plan itself. Do NOT modify, create, or delete any other files, and do NOT run commands that change system or repository state. When you have enough understanding, present findings and a concrete plan.`,
    blockNonMdWrites: true,
  },
];

let modes: ModeDef[] = DEFAULT_MODES.map((m) => ({ ...m }));
let current: string = "build";

export function setModes(defs: ModeDef[]): void {
  modes = defs.map((m) => ({ ...m }));
  if (!modes.some((m) => m.name === current)) current = modes[0]?.name ?? "build";
}

/** Merge user config over defaults: same name overrides, new names append. */
export function loadModesFromConfigFile(path: string): boolean {
  try {
    // Lazy require-free JSON read keeps this importable anywhere.
    const src = (globalThis as { __officinaReadText?: (p: string) => string }).__officinaReadText?.(path)
      ?? readTextFileSync(path);
    if (!src) return false;
    const parsed = JSON.parse(src);
    const list: ModeDef[] = Array.isArray(parsed?.modes) ? parsed.modes : [];
    if (list.length === 0) return false;
    const merged = [...modes];
    for (const def of list) {
      if (!def?.name) continue;
      const i = merged.findIndex((m) => m.name === def.name);
      if (i >= 0) merged[i] = { ...merged[i], ...def };
      else merged.push({ ...def });
    }
    setModes(merged);
    return true;
  } catch {
    return false; // bad config file must never break mode switching
  }
}

function readTextFileSync(path: string): string | null {
  try {
    return readFileSync(path, "utf-8");
  } catch {
    return null;
  }
}

export function modesPath(env: NodeJS.ProcessEnv = process.env): string {
  return env.OFFICINA_MODES
    || join(env.HOME || homedir(), ".vitriol", "officina", "modes.json");
}

export function setAgentMode(name: string): boolean {
  if (!modes.some((m) => m.name === name)) return false;
  current = name;
  return true;
}

export function getAgentMode(): string {
  return current;
}

export function getModeDef(name?: string): ModeDef {
  return modes.find((m) => m.name === (name ?? current)) ?? modes[0];
}

export function allModes(): ModeDef[] {
  return modes;
}

/** Next mode in definition order (TAB cycling). */
export function nextMode(from?: string): ModeDef {
  const i = modes.findIndex((m) => m.name === (from ?? current));
  return modes[(i + 1) % modes.length];
}

/** Resolve a mode color to an SGR foreground sequence (raw, no reset). */
export function modeColorSeq(def: ModeDef): string {
  if (def.color.startsWith("#")) {
    const { r, g, b } = hexToRgb(def.color);
    return `\x1b[38;2;${r};${g};${b}m`;
  }
  const hex = (VITRIOLUM as Record<string, string>)[def.color];
  if (!hex) return "";
  const { r, g, b } = hexToRgb(hex);
  return `\x1b[38;2;${r};${g};${b}m`;
}
