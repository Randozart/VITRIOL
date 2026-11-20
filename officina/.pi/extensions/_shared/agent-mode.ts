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
    name: "grind",
    label: "GRIND",
    glyph: "∞",
    color: "violet", // alchemical violet: the work that does not stop
    hint: "persistent build · runs until GRIND: DONE · TAB / /agent build",
    directive: `AGENT MODE: GRIND (persistent build — keep going until the work is truly done).
You are allowed to modify files and run state-changing commands. Work the task list to zero: if the request implies more than one step and no task list exists, create one first (update_tasks), then execute every item. Do NOT stop at a natural checkpoint, do NOT ask permission for in-scope work, and do NOT end the turn with only a summary while work remains — continue into the next step yourself. Verify each change (build/tests/gates) before claiming it done. You may end the turn only with a final line of exactly one of:
  GRIND: DONE — only when the task list has no pending or in-progress items and your changes are verified;
  GRIND: BLOCKED: <one-line reason> — only when progress is impossible without the user (missing decision, credentials, or access).
If you stop without one of those markers, the harness sends you a continuation message. If the user sends any input while the loop runs, the loop stands down and follows them.`,
    enterDirective: `GRIND armed: the continuation loop is active — keep working until the task list is empty and your changes are verified, then finish with GRIND: DONE (or GRIND: BLOCKED: <reason>).`,
  },
  {
    name: "steer",
    label: "STEER",
    glyph: "↝",
    color: "antidote", // copper/orange: course corrections
    hint: "redirect the work in flight · keep going after the correction · TAB / /agent build",
    directive: `AGENT MODE: STEER (course correction, keep working).
The user is steering the work in progress. Apply the correction to what you are ALREADY doing — adjust the approach, the target, or the emphasis — and then continue the task; do not stop to re-plan and do not start unrelated work. If the "correction" is really a question, answer it in one or two sentences and resume.`,
  },
  {
    name: "inquire",
    label: "INQUIRE",
    glyph: "?",
    color: "safety", // green: confirmations/steering are constructive, non-blocking
    hint: "answers & steering only · work keeps running · TAB / /agent build",
    directive: `AGENT MODE: INQUIRE (confirm & steer, do not act).
Answer the user's question directly and concisely, and confirm or correct the current approach when asked. Do NOT start new work for this turn: no file modifications, no state-changing commands. If the request is really an instruction to change something, answer/piece the plan and tell the user to switch to BUILD (TAB). Keep answers short — full explanations go in the answer panel.`,
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
  {
    name: "conversation",
    label: "CONVERSATION",
    glyph: "◇",
    color: "#2fa08a", // sea-glass teal: discussion water, not excavation
    hint: "repo chat & light research · no state changes · TAB / /agent build",
    directive: `AGENT MODE: CONVERSATION (discuss, do not act).
This is a discussion turn about the repo, the architecture, or lightweight
research. Answer concisely and substantively; you may read files and browse
the web to ground the discussion. Do NOT modify files, run state-changing
commands, or create task lists. If the discussion crystallises into a change
request, say so and suggest switching to PLAN (TAB).`,
  },
  {
    name: "research",
    label: "RESEARCH",
    glyph: "∿",
    color: "#3a74d4", // abyss blue, lit past the 3:1 badge gate (distinct from BUILD)'s coldBlue
    hint: "deep online research · *.md writes only · TAB / /agent plan",
    directive: `AGENT MODE: RESEARCH (deep research-first, may go online).
Investigate the question THOROUGHLY: the repo first (code, tests, docs, prior
plans), then the wider world — best architectural practice, how comparable
systems solve it, trade-offs and failure modes. Prefer the deep-research
pipeline (its Scope→Research fan-out with websearch/webfetch/browser and NO
bash in research children) over ad-hoc browsing when the question is broad.
Cite what you found (file:line for local, URL for web). You may ONLY create
or modify Markdown files — research notes in .opencode/plans/. Do NOT modify
any other files and do NOT run state-changing commands. End by OFFERING the
formalisation: "switch to PLAN to turn this into a plan, or PIPELINE to run
research→plan→confirm→grind end-to-end" — then stop and let the owner pick.`,
    blockNonMdWrites: true,
  },
  {
    name: "pipeline",
    label: "PIPELINE",
    glyph: "⟲",
    color: "#e05adf", // philosopher's ember: between GRIND violet and rubedo red
    hint: "spec-driven: research → confirm → plan → grind · compacts after",
    directive: `AGENT MODE: PIPELINE (spec-driven development, end-to-end).
Run the full chain, in order, with HARD GATES between phases:
  1. RESEARCH — investigate deeply (repo + online best practice, deep-research
     pipeline for broad questions). Cite findings.
  2. CONFIRM GATE — present the findings and the proposed plan to the owner
     and STOP. Do not proceed on your own. Resume only after the owner
     confirms; when they do, state the marker line exactly:
     PIPELINE: PLAN CONFIRMED
  3. PLAN — write the plan to .opencode/plans/ with an ## Invariants section
     (numbered properties, each tied to the test that pins it) and named
     tests that would prove the work correct.
  4. GRIND — switch mentally to persistent-build rules: create the task list,
     implement to zero, verify each change with the tests the plan defined,
     commit per stage. Do not stop at checkpoints; end only with
     GRIND: DONE or GRIND: BLOCKED: <reason>.
  5. COMPACT — after GRIND: DONE, the compact-on-finish loop compacts as
     usual; nothing extra is required.
If the owner sends any input mid-pipeline, it stands down and follows them
(same as GRIND). Never skip phase 2: implementing without the owner's
explicit confirmation is a violation of this mode, not an efficiency.`,
  },
];

let modes: ModeDef[] = DEFAULT_MODES.map((m) => ({ ...m }));
let current: string = "build";

export function setModes(defs: ModeDef[]): void {
  modes = defs.map((m) => ({ ...m }));
  if (!modes.some((m) => m.name === current))
    current = modes[0]?.name ?? "build";
}

/** Merge user config over defaults: same name overrides, new names append. */
export function loadModesFromConfigFile(path: string): boolean {
  try {
    // Lazy require-free JSON read keeps this importable anywhere.
    const src =
      (
        globalThis as { __officinaReadText?: (p: string) => string }
      ).__officinaReadText?.(path) ?? readTextFileSync(path);
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
  return (
    env.OFFICINA_MODES ||
    join(env.HOME || homedir(), ".vitriol", "officina", "modes.json")
  );
}

/** Facts file the TUI/other extensions read for the CURRENT mode (jiti
 *  isolation: module state does not cross extension boundaries). */
export function agentModeFactsPath(
  env: NodeJS.ProcessEnv = process.env,
): string {
  return join(env.HOME || homedir(), ".vitriol", "officina", "agent-mode.json");
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
