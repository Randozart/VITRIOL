import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { createBelowEditorFallback, registerSidebarSection, requestSidebarUpdate } from "../_shared/sidebar.ts";
import { renderReport, type GateObs, type VerifyResult } from "./baseline.ts";
import { detectGates, type Gate, type VerifyConfig } from "./gates.ts";
import { AUTO_BUDGET_MS, normalizeTouched, parseVerifyArgs, runVerify, type ExecFn } from "./runner.ts";
import { renderWidgetLines } from "./widget.ts";

// verify — the standards tool (plan: officina-qol-batch, stage 1).
//
// The project's verification lives IN the tool: manifest-driven gate
// discovery (+ VITRIOL signature special-case), session-start cheap
// baseline, classification into pre-existing / in-progress / regression
// (I1 — half-implemented features never show as red; see baseline.ts).
// Advisory-only (I2): never edits, never blocks. I3: the auto pass is
// fire-and-forget with a wall budget. Surfaces: pi tool `verify`,
// `/verify [cheap|full] [--strict]` dialog, ANSI sidebar/status widget.
// Kill switch: OFFICINA_NO_VERIFY=1.

export default function (pi: ExtensionAPI, deps?: { exec?: ExecFn }) {
  if (process.env.OFFICINA_NO_VERIFY === "1") return;

  let cwd = process.cwd();
  // eslint-disable-next-line @typescript-eslint/no-explicit-any -- ctx-bound ui, session_start
  let ui: any = null;
  let gates: Gate[] = [];
  let config: VerifyConfig = { auto: true, disable: [] };
  const baseline = new Map<string, GateObs>();
  const touched = new Set<string>();
  let last: VerifyResult | null = null;
  let running: string[] = []; // gate IDs currently executing
  let runSeq = 0;

  const runningLabels = (): string[] =>
    running.map((id) => gates.find((g) => g.id === id)?.label ?? id);
  const getLines = () => renderWidgetLines(last, runningLabels());
  const fallback = createBelowEditorFallback("verify", getLines);

  const render = (): void => {
    requestSidebarUpdate();
    if (ui) fallback(ui);
  };

  const detect = (): void => {
    const d = detectGates(cwd);
    gates = d.gates;
    config = d.config;
  };

  interface RunHandle {
    done: Promise<{ busy?: boolean; stale?: boolean; result?: VerifyResult }>;
  }

  /** Shared run flow for tool / command / auto pass. */
  function startRun(tier: "cheap" | "full", strict: boolean, auto = false): RunHandle {
    if (running.length > 0) {
      return { done: Promise.resolve({ busy: true }) };
    }
    const seq = ++runSeq;
    const wanted = gates.filter((g) => tier === "full" || g.tier === "cheap");
    running = wanted.map((g) => g.id);
    render();
    const done = runVerify(gates, {
      tier,
      strict,
      baseline,
      touched,
      exec: deps?.exec,
      budgetMs: auto ? AUTO_BUDGET_MS : undefined,
      stale: () => seq !== runSeq,
      onGate: (gate) => {
        if (seq !== runSeq) return;
        running = running.filter((id) => id !== gate.id);
        render();
      },
    }).then((result) => {
      if (seq !== runSeq) return { stale: true as const };
      running = [];
      last = result;
      render();
      emitHarnessEvent(
        harnessEvent("lc-verify", tier, {
          detail: `gates=${result.outcomes.length} regs=${result.regressions.length} prog=${result.inProgress.length} pre=${result.preExisting.length} tookMs=${result.tookMs}`,
        }),
      );
      if (ui && result.regressions.length > 0) {
        ui.notify(`verify: ${result.regressions.length} regression — /verify`, "warning");
      }
      return { result };
    });
    return { done };
  }

  registerSidebarSection("verify", 55, () => getLines());

  pi.on("session_start", (_event, ctx) => {
    const c = (ctx as { cwd?: string }).cwd;
    if (c) cwd = c;
    ui = ctx.hasUI ? ctx.ui : null;
    // session swap = fresh verification world (bash-title precedent)
    runSeq++;
    running = [];
    last = null;
    baseline.clear();
    touched.clear();
    detect();
    if (config.auto && gates.some((g) => g.auto)) {
      void startRun("cheap", false, true); // I3: fire-and-forget, never blocks start
    }
    render();
  });

  pi.on("tool_result", (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    if (e.isError) return;
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (file) touched.add(normalizeTouched(file, cwd));
  });

  pi.registerTool({
    name: "verify",
    label: "Verify",
    description:
      "Run the project's verification gates (typecheck, lint/fmt, tests — detected from manifests) " +
      "with baseline classification: regressions (new, files you were NOT editing) vs in-progress " +
      "(new, files you touched — expected mid-work) vs pre-existing (failed at session start). " +
      "Use after finishing a feature or before committing; cheap=typecheck/lint only.",
    parameters: Type.Object({
      tier: Type.Optional(Type.Union([Type.Literal("cheap"), Type.Literal("full")], { description: "cheap = typecheck/lint/fmt only (default full)" })),
      strict: Type.Optional(Type.Boolean({ description: "classify in-progress diagnostics as regressions" })),
    }),
    async execute(_id, params) {
      if (gates.length === 0) detect();
      if (gates.length === 0) {
        return { content: [{ type: "text" as const, text: "verify: no gates detected in this project (manifests: package.json, Cargo.toml, pyproject, go.mod)." }], details: {} };
      }
      const tier = params.tier ?? "full";
      const strict = params.strict ?? false;
      const { done } = startRun(tier, strict);
      const out = await done;
      if (out.busy) {
        return { content: [{ type: "text" as const, text: "verify: already running — wait for it (widget shows progress)." }], details: {} };
      }
      if (out.stale || !out.result) {
        return { content: [{ type: "text" as const, text: "verify: session changed mid-run — result discarded, re-run." }], details: {} };
      }
      return { content: [{ type: "text" as const, text: renderReport(out.result).join("\n") }], details: {} };
    },
  });

  pi.registerCommand("verify", {
    description: "Run verification gates: /verify [cheap|full] [strict] — full report dialog",
    handler: async (args: string, ctx: { ui: any }) => {
      if (gates.length === 0) detect();
      if (gates.length === 0) {
        await ctx.ui.notify?.("verify: no gates detected", "warning");
        return;
      }
      const { tier, strict } = parseVerifyArgs(args);
      const { done } = startRun(tier, strict);
      const out = await done;
      if (out.busy) {
        await ctx.ui.notify?.("verify: already running", "warning");
        return;
      }
      if (out.stale || !out.result) {
        await ctx.ui.notify?.("verify: session changed mid-run — discarded", "warning");
        return;
      }
      const report = renderReport(out.result).join("\n");
      try {
        await ctx.ui.select?.(report, ["close"]);
      } catch {
        // dialog dismissed — the widget still holds the summary
      }
    },
  });
}
