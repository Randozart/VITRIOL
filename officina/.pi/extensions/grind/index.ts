// GRIND loop (2026-11-14) — makes the GRIND agent mode actually persistent.
//
// A directive alone cannot continue work: pi ends a turn when the model stops
// calling tools. This extension re-prompts on agent_end until the model proves
// completion (`GRIND: DONE`), reports a block (`GRIND: BLOCKED: <reason>`), or
// a guard fires. Same mechanic as fix-loop; deliberately separate so the
// mode registry stays data-only.
//
// Discipline:
//   - strict stop: DONE counts only with an empty task list and green gates
//     (one reminder, then accepted — no fight);
//   - user wins: input while the loop runs stands it down;
//   - abort (Esc) never re-prompts (agent_settled.aborted);
//   - bounded: OFFICINA_GRIND_MAX_TURNS (24, hard 100) and
//     OFFICINA_GRIND_MAX_MINUTES (120);
//   - one continuation per settled turn; skips while queued/compacting/retry;
//   - cache-safe: continuation is a tail user message (sendUserMessage).
// Kill switch: OFFICINA_NO_GRIND=1.

import { readFileSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { agentModeFactsPath } from "../_shared/agent-mode.ts";
import {
  decide,
  freshState,
  grindDisabled,
  activitySignature,
  maxMinutes,
  maxTurns,
  type GrindState,
  type TaskSnapshot,
} from "./rules.ts";
import { getTaskItems, getTaskSummary } from "../task-state/index.ts";
import { detectGates } from "../verify/gates.ts";
import { runVerify } from "../verify/runner.ts";
import type { GateObs, VerifyResult } from "../verify/baseline.ts";

type LoopCtx = {
  ui?: { notify?: (text: string, kind?: string) => unknown };
  cwd?: string;
  sendUserMessage?: (text: string) => Promise<void>;
};

const WIDGET_KEY = "grind";

function readMode(): string | null {
  try {
    const doc = JSON.parse(readFileSync(agentModeFactsPath(), "utf8")) as { mode?: unknown };
    return typeof doc.mode === "string" ? doc.mode : null;
  } catch {
    return null;
  }
}

function assistantTextOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => {
      const p = part as { type?: string; text?: unknown };
      return p?.type === "text" ? String(p.text ?? "") : "";
    })
    .join("");
}

/** Abort often lands as agent_end before agent_settled — the assistant
 *  message carries stopReason "aborted" (I3: never re-prompt after Esc). */
function lastAssistantAborted(messages: unknown): boolean {
  if (!Array.isArray(messages)) return false;
  for (let i = messages.length - 1; i >= 0; i--) {
    const msg = messages[i] as { role?: string; stopReason?: string };
    if (msg?.role !== "assistant") continue;
    return msg.stopReason === "aborted";
  }
  return false;
}

function taskSnapshot(): TaskSnapshot | null {
  const summary = getTaskSummary();
  if (!summary) return null;
  const open = getTaskItems()
    .filter((t) => t.status === "pending" || t.status === "in_progress")
    .map((t) => t.description);
  return { pending: summary.pending, inProgress: summary.inProgress, open };
}

export default function (pi: ExtensionAPI) {
  if (grindDisabled()) return;

  let state: GrindState | null = null;
  let disarmed = false;
  let compacting = false;
  let queued = false;
  let verifying = false;

  const paint = (ctx: LoopCtx, text: string | null): void => {
    const ui = ctx.ui as { setWidget?: (key: string, lines: string[], opts?: unknown) => unknown };
    try {
      ui?.setWidget?.(WIDGET_KEY, text ? [text] : [], { placement: "belowEditor" });
    } catch {
      // decoration contract
    }
  };

  const notify = (ctx: LoopCtx, text: string, kind: "info" | "warning" = "info"): void => {
    try {
      void ctx.ui?.notify?.(text, kind);
    } catch {
      // decoration contract
    }
  };

  const verifyOnce = async (cwd: string): Promise<{ ran: boolean; failed: string[] }> => {
    const { gates, config } = detectGates(cwd);
    const usable = gates.filter((g) => !config.disable.includes(g.id));
    if (usable.length === 0) return { ran: false, failed: [] };
    const result: VerifyResult = await runVerify(usable, {
      tier: "full",
      strict: false,
      baseline: new Map<string, GateObs>(),
      touched: new Set<string>(),
      budgetMs: undefined,
    });
    const failed = [...result.regressions, ...result.inProgress].map(
      (c) => `${c.gateId}: ${String(c.diag?.message ?? "").slice(0, 120)}`,
    );
    return { ran: true, failed };
  };

  let lastText = "";
  let abortedSeen = false;

  pi.on("input", (event) => {
    if (event.source === "extension") return;
    if (event.streamingBehavior) queued = true;
    if (state) {
      // The user interjected while the loop was running: stand down for
      // this round (they own the next turn).
      state = null;
      disarmed = true;
    } else {
      // A fresh user instruction re-arms the loop for its settle.
      disarmed = false;
    }
  });

  pi.on("message_end", (event) => {
    const msg = event.message as { role?: string; content?: unknown; stopReason?: string };
    if (msg?.role !== "assistant") return;
    lastText = assistantTextOf(msg.content);
    abortedSeen = msg.stopReason === "aborted";
  });

  // agent_settled is the documented "no automatic retry, compaction, or
  // queued continuation will run" point — the right place to decide.
  pi.on("agent_settled", async (event, ctx) => {
    const loopCtx = ctx as unknown as LoopCtx;
    const mode = readMode();
    if (mode !== "grind") {
      state = null;
      disarmed = false;
      paint(loopCtx, null);
      lastText = "";
      abortedSeen = false;
      queued = false;
      return;
    }
    if (disarmed) return;
    if (verifying) return;

    const text = lastText;
    lastText = "";
    const aborted = event.aborted || abortedSeen;
    abortedSeen = false;
    const wasQueued = queued;
    queued = false;

    const tasks = taskSnapshot();
    const now = Date.now();
    const current = state ?? freshState(now);
    const round = current.rounds + 1;

    if (aborted) {
      state = null;
      paint(loopCtx, null);
      return;
    }

    // Verify on a DONE claim or every third continuation (operator choice).
    const terminalClaim = /\bGRIND:\s*DONE\b/i.test(text);
    let verify: { ran: boolean; failed: string[] } | null = null;
    if (terminalClaim || round % 3 === 0) {
      verifying = true;
      try {
        verify = await verifyOnce(String(loopCtx.cwd ?? process.cwd()));
      } catch {
        verify = null;
      } finally {
        verifying = false;
      }
    }

    const decision = decide({
      mode,
      aborted,
      userDisarmed: disarmed,
      queued: wasQueued,
      compacting: false,
      willRetry: false, // settled implies no automatic retry is pending
      assistantText: text,
      tasks,
      verify,
      state: current,
      now,
      maxTurns: maxTurns(),
      maxMinutes: maxMinutes(),
      signature: activitySignature(tasks),
    });

    if (decision.action === "continue") {
      state = decision.next;
      paint(loopCtx, `↻ grind ${decision.round}/${maxTurns()}`);
      try {
        await loopCtx.sendUserMessage?.(decision.prompt);
      } catch (error) {
        state = null;
        notify(
          loopCtx,
          `grind: could not send the continuation (${error instanceof Error ? error.message : String(error)}) — loop stopped`,
          "warning",
        );
        paint(loopCtx, null);
      }
      return;
    }

    state = null;
    paint(loopCtx, null);
    if (decision.reason === "mode-changed" || decision.reason === "user-input") return;
    const kind = decision.reason === "done" ? "info" : "warning";
    notify(loopCtx, `grind: ${decision.note}`, kind as "info" | "warning");
  });

  pi.on("session_start", (_event, ctx) => {
    state = null;
    disarmed = false;
    paint(ctx as unknown as LoopCtx, null);
  });
}
