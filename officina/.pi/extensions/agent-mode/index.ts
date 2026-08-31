// agent-mode (2026-08-31): Plan/Build agent modes, new in Officina.
// Provenance: original work, this repo (First-Party Mandate). See header
// block below for behavior and limits.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { setAgentMode, type AgentMode } from "../_shared/agent-mode.ts";
import { fgSeq } from "../_shared/vitriolum.ts";
import { injectionResult } from "../_shared/inject.ts";

// agent-mode (2026-08-31, owner request): Plan / Build agent modes.
//
//   /mode            show current mode
//   /mode plan       research mode: explicit research directive injected;
//                    write/edit tool calls BLOCKED except *.md files
//   /mode build      work mode: write gate removed + one-shot hint that
//                    the agent is allowed to modify files again
//
// Directive travels via _shared/inject (cache-safe tail message, Rule 7).
// Enforcement blocks the write/edit TOOLS on non-.md targets. bash is NOT
// parsed for mutations (unreliable); the plan directive tells the model
// not to mutate — governance is belt+braces, not theater. Honest limit.
//
// Kill switch: OFFICINA_AGENT_MODE=0. Indicator widget key: "agent-mode".

type Mode = "build" | "plan";

const PLAN_DIRECTIVE = `AGENT MODE: PLAN (research-first).
Behavior for this mode: investigate before acting — read the relevant code, trace the call paths, check tests and docs. Think about approaches and trade-offs. You may ONLY create or modify Markdown files (*.md) — use them for notes, findings, and the plan itself. Do NOT modify, create, or delete any other files, and do NOT run commands that change system or repository state. When you have enough understanding, present findings and a concrete plan.`;

const BUILD_HINT = `AGENT MODE: BUILD.
Plan mode has ended — you are allowed to modify files and run state-changing commands again, effective immediately. Apply the plan (or the current request) with normal write/edit tools.`;

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_AGENT_MODE === "0") return; // Rule 15

  let mode: Mode = "build";
  let ui: any = null;

  // Indicator discipline (owner request 2026-08-31): the mode must be
  // unambiguous at a glance in BOTH directions — the absence of a PLAN
  // banner was not proof of BUILD. Both states render, always BOLD; PLAN
  // is additionally loud (Vitriolum antidote orange), BUILD quiet but
  // present (muted gray). The session-panel sidebar mirrors the badge via
  // _shared/agent-mode.ts.
  const BOLD = "\x1b[1m";
  const RESET = "\x1b[0m";
  const renderIndicator = () => {
    // ui is ctx-bound (set on session_start); widgets never capture input.
    if (!ui) return;
    if (mode === "plan") {
      ui.setWidget?.(
        "agent-mode",
        [`${BOLD}${fgSeq("antidote")}► PLAN MODE — research only, *.md writes · TAB / /mode build${RESET}`],
        { placement: "belowEditor" },
      );
    } else {
      ui.setWidget?.(
        "agent-mode",
        [`${BOLD}${fgSeq("gray")}▪ build mode · TAB / /mode plan for research${RESET}`],
        { placement: "belowEditor" },
      );
    }
  };

  // One-shot: after plan -> build, hint the model ONCE that writes are
  // allowed again. Delivered as a hidden ride-along on the user's NEXT
  // turn (beforeAgentStart) - never via sendUserMessage, which would
  // start a turn on its own (the "switch fires inference" bug).
  let buildHintPending = false;

  const setMode = (next: Mode, ctx?: { ui?: any }) => {
    if (next === mode) {
      ctx?.ui?.notify?.(`agent mode: already ${mode}`, "info");
      return;
    }
    mode = next;
    setAgentMode(next as AgentMode);
    if (mode === "plan") {
      ctx?.ui?.notify?.("PLAN mode: research only, *.md writes", "info");
    } else {
      buildHintPending = true;
      ctx?.ui?.notify?.("BUILD mode: writes unblocked", "info");
    }
    renderIndicator();
  };

  // Ride-along delivery: attach mode directives to the user's own turn.
  // In plan mode the directive rides EVERY turn; the build hint rides
  // exactly once, then is consumed.
  pi.on("before_agent_start", () => {
    if (mode === "plan") {
      return injectionResult("agent-mode", PLAN_DIRECTIVE);
    }
    if (buildHintPending) {
      buildHintPending = false;
      return injectionResult("agent-mode", BUILD_HINT);
    }
    return;
  });

  pi.on("session_start", (_event, ctx) => {
    ui = ctx.ui;
    renderIndicator();
  });

  // TAB toggles Plan/Build (owner request 2026-08-31). TAB also drives
  // editor autocomplete (tui.input.tab); the mode toggle is the more
  // valuable binding on this harness — pass-through autocomplete still
  // exists via explicit completion UI. Default: Build; TAB flips to Plan.
  pi.registerShortcut("tab", {
    description: "Toggle Plan / Build agent mode",
    handler: async (ctx: { ui?: any }) => {
      setMode(mode === "build" ? "plan" : "build", ctx);
    },
  });

  pi.registerCommand("mode", {
    description: "Agent mode: TAB or /mode plan|build — plan = research, *.md-only writes; build = full writes",
    handler: async (args: string, ctx: { ui: any }) => {
      const a = args.trim().toLowerCase();
      if (!a) {
        await ctx.ui.notify?.(`agent mode: ${mode}`, "info");
        return;
      }
      if (a !== "plan" && a !== "build") {
        await ctx.ui.notify?.("usage: /mode plan | /mode build", "warning");
        return;
      }
      setMode(a as Mode, ctx);
    },
  });

  // Enforcement: block non-.md writes while in plan mode.
  pi.on("tool_call", (event) => {
    if (mode !== "plan") return;
    if (event.toolName !== "write" && event.toolName !== "edit") return;
    const input = (event as { input?: Record<string, unknown> }).input ?? {};
    const path = String(input.file_path ?? input.path ?? "");
    if (/\.md$/i.test(path)) return; // markdown writes are the point of plan mode
    return {
      block: true,
      reason:
        "PLAN MODE is active: only *.md files may be written. Finish your research/plan, or run /mode build to unlock writes.",
    } as never;
  });

  renderIndicator();
}
