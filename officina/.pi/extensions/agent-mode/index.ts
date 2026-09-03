// agent-mode (2026-08-31): Plan/Build agent modes, new in Officina.
// Provenance: original work, this repo (First-Party Mandate).

// agent-mode (owner request 2026-09-01): modes are now CONFIGURABLE.
//
//   /mode            show current mode + registered modes
//   /mode <name>     switch to a named mode (TAB cycles)
//
// Mode definitions live in _shared/agent-mode.ts (defaults: build coldBlue /
// plan gold) and can be overridden or extended via
// ~/.vitriol/officina/modes.json (schema documented there). Every surface
// (widget, sidebar badge, session-panel frame edge) renders from the same
// registry.
//
// Directive travels via _shared/inject (cache-safe tail message, Rule 7).
// Per-mode write gates: blockNonMdWrites blocks write/edit on non-.md
// targets. bash is NOT parsed for mutations (unreliable); the directive
// tells the model not to mutate — governance is belt+braces, not theater.
//
// Kill switch: OFFICINA_AGENT_MODE=0. Indicator widget key: "agent-mode".

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  allModes,
  getModeDef,
  loadModesFromConfigFile,
  modeColorSeq,
  modesPath,
  nextMode,
  setAgentMode,
} from "../_shared/agent-mode.ts";
import { VITRIOLUM, fgSeq, hexToRgb } from "../_shared/vitriolum.ts";
import { injectionResult } from "../_shared/inject.ts";

const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_AGENT_MODE === "0") return; // Rule 15

  let ui: any = null;

  // Owner-configurable modes: same-name overrides, new names append.
  loadModesFromConfigFile(modesPath());
  setAgentMode(allModes()[0]?.name ?? "build");

  /** Widget line for a quiet mode: BOLD glyph+label, hint in muted. */
  const badgeLine = (def: ReturnType<typeof getModeDef>): string => {
    const base = `${BOLD}${modeColorSeq(def)}${def.glyph} ${def.label}${RESET}`;
    const hint = `${fgSeq("gray")} · ${def.hint} · TAB / /mode ${nextMode(def.name).name}${RESET}`;
    return base + hint;
  };

  // Mode indicator lives BELOW the editor — canonical spot (owner decision
  // 2026-09-01): bold glyph+label in the mode color, recoloring on every
  // mode switch, next to the composer where the eye is. Sidebar carries
  // data rows only; the brief sidebar-section experiment was reverted.
  const renderIndicator = () => {
    if (!ui) return;
    const def = getModeDef();
    const defNext = nextMode(def.name);
    // Loud (directive) modes paint the whole hint line in the mode color.
    const line = def.directive
      ? `${BOLD}${modeColorSeq(def)}${def.glyph} ${def.label} MODE — ${def.hint} · TAB / /mode ${defNext.name}${RESET}`
      : badgeLine(def);
    ui.setWidget?.("agent-mode", [line], { placement: "belowEditor" });
  };

  // One-shot: after switching INTO a mode with an enterDirective, hint the
  // model ONCE on the user's NEXT turn (beforeAgentStart) — never via
  // sendUserMessage, which would start a turn on its own (the "switch fires
  // inference" bug).
  let enterDirectivePending: string | null = null;

  const setMode = (name: string, ctx?: { ui?: any }) => {
    const def = getModeDef(name);
    if (def.name === getModeDef().name) {
      ctx?.ui?.notify?.(`agent mode: already ${def.name}`, "info");
      return;
    }
    if (!setAgentMode(name)) {
      ctx?.ui?.notify?.(`agent mode: unknown mode "${name}"`, "warning");
      return;
    }
    if (def.enterDirective) enterDirectivePending = def.enterDirective;
    // No notify here (owner request 2026-09-02): the mode chip above the
    // prompt box + border tint already announce the switch; the footer
    // notice duplicated it as noise.
    applyModeTheme();
    renderIndicator();
  };

  // Ride-along delivery: the current mode's directive rides EVERY turn;
  // an enterDirective rides exactly once, then is consumed.
  pi.on("before_agent_start", () => {
    const def = getModeDef();
    if (def.directive) {
      return injectionResult("agent-mode", def.directive);
    }
    if (enterDirectivePending) {
      const d = enterDirectivePending;
      enterDirectivePending = null;
      return injectionResult("agent-mode", d);
    }
    return;
  });

  // Mode-tinted chrome (owner request 2026-09-01): the chat box border
  // follows the active mode's color. Done by swapping in a clone of the
  // officina theme with the "border" role recolored — pi already rebuilds
  // the editor border on theme change (updateEditorBorderColor), so this
  // rides the supported path. Cosmetic: every failure is swallowed.
  const applyModeTheme = () => {
    // Publish the border tint for the patched TUI (updateEditorBorderColor
    // reads it after the thinking-level border logic).
    try {
      const def2 = getModeDef();
      const seq = modeColorSeq(def2);
      // pi editors call borderColor(text) — publish a styled function, not
      // a raw sequence (2026-09-01: string tint crashed CustomEditor.render).
      (globalThis as any).__officinaModeBorder = seq
        ? ((text: string) => `${seq}${text}${RESET}`)
        : null;
    } catch { /* cosmetic */ }
    try {
      const def = getModeDef();
      const hex = def.color.startsWith("#") ? def.color : (VITRIOLUM as Record<string, string>)[def.color];
      if (!hex) return;
      const base = ui.getTheme?.("officina") ?? ui.theme;
      console.error("[officina] applyModeTheme base:", !!base, "fgColors:", !!base?.fgColors, "setTheme:", typeof ui.setTheme);
      if (!base?.fgColors) return;
      const { r, g, b } = hexToRgb(hex);
      const clone = Object.create(Object.getPrototypeOf(base));
      Object.assign(clone, base);
      clone.fgColors = new Map(base.fgColors);
      clone.fgColors.set("border", `[38;2;${r};${g};${b}m`);
      clone.name = `officina-${def.name}`;
      ui.setTheme?.(clone);
    } catch (err) {
      console.error("[officina] applyModeTheme failed:", err instanceof Error ? err.message : err);
    }
  };

  pi.on("session_start", (_event, ctx) => {
    ui = ctx.ui;
    applyModeTheme();
    renderIndicator();
  });

  // TAB cycles through the registered modes in definition order.
  pi.registerShortcut("tab", {
    description: "Cycle agent modes (Plan / Build / custom)",
    handler: async (ctx: { ui?: any }) => {
      setMode(nextMode().name, ctx);
    },
  });

  pi.registerCommand("mode", {
    description: "Agent mode: TAB or /mode <name> — /mode alone lists modes",
    handler: async (args: string, ctx: { ui: any }) => {
      const a = args.trim().toLowerCase();
      if (!a) {
        const list = allModes().map((m) => m.name).join(", ");
        await ctx.ui.notify?.(`agent mode: ${getModeDef().name} (modes: ${list})`, "info");
        return;
      }
      setMode(a, ctx);
    },
  });

  // Enforcement: per-mode write gate (plan blocks non-.md writes).
  pi.on("tool_call", (event) => {
    const def = getModeDef();
    if (!def.blockNonMdWrites) return;
    if (event.toolName !== "write" && event.toolName !== "edit") return;
    const input = (event as { input?: Record<string, unknown> }).input ?? {};
    const path = String(input.file_path ?? input.path ?? "");
    if (/\.md$/i.test(path)) return; // markdown writes are the point of plan mode
    return {
      block: true,
      reason:
        `${def.label} MODE is active: only *.md files may be written. Finish your research/plan, or run /mode ${allModes()[0]?.name ?? "build"} to unlock writes.`,
    } as never;
  });

  renderIndicator();
}
