// PROVENANCE: inspiration — intel/intel-ai-builder (Apache-2.0), SuperClaw
// "Auto Route": classify every task (complexity + data sensitivity) before the
// model call and route across execution tiers with a user-tunable
// quality-vs-cost knob. Intel's router is closed-source; only the documented
// architecture (README, newsroom article, The Neuron podcast 2026-08-05)
// informed this design. Implementation is original VITRIOL work: classification
// heuristics, route table, and the cloud-as-co-processor delivery are ours.
//
// auto-route — per-turn task routing for officina.
//
// Fires on `context` (before every LLM call). Classifies the turn via
// classifier.ts, routes via router.ts. Three modes:
//   suggest (default) — status line only, user stays in control
//   auto              — hard/safe turns are escalated through ascensusd and
//                       the cloud verdict is injected as a tail message; the
//                       local model answers WITH the verdict in context
//   off               — dead extension
//
// Why injection instead of pi.setModel for the cloud tier: a model switch on a
// single local llama.cpp box evicts weights (15s+ stall, see phase-model
// notes), and a chat-completions cloud provider would bypass ascensusd's
// euro-budget single-writer. Injecting the ascensusd answer keeps budget
// accounting in exactly one place and the local model authoritative.
//
// Layer discipline: injection is an append-only tail custom message — cache
// safe by construction (Rule 7), same mechanism as context-relay/scratchpad.
//
// Kill switch: OFFICINA_NO_AUTO_ROUTE=1 or OFFICINA_ROUTE_MODE=off (Rule 15).

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { readFileSync } from "node:fs";
import { basename } from "node:path";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { classifyTurn, scratchpadContextLines, type ClassifyInput } from "./classifier.ts";
import { resolveMode, resolveThreshold, route } from "./router.ts";

const ASCENSUS_URL =
  process.env.OFFICINA_ROUTE_ASCENSUS_URL || "http://127.0.0.1:8283";
const SCRATCHPAD_FILE =
  process.env.OFFICINA_ROUTE_SCRATCHPAD || ".officina/SCRATCHPAD.md";
/** Queries shorter than this never escalate — routing is for real work. */
const MIN_QUERY_CHARS = 24;
/** How many trailing toolResults to scan for recent failures. */
const ERROR_SCAN_WINDOW = 6;
/** events.jsonl lines scanned for recent lc-churn loop firings. */
const CHURN_SCAN_LINES = 200;
const CHURN_WINDOW_S = 1800;

interface ToolResultShape {
  role?: string;
  toolCallId?: string;
  isError?: boolean;
}

function isToolResult(m: unknown): m is ToolResultShape {
  return typeof m === "object" && m !== null && (m as { role?: string }).role === "toolResult";
}

function textOf(content: unknown): string {
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const c of content as Array<{ type?: string; text?: string }>) {
    if (c?.type === "text" && typeof c.text === "string") parts.push(c.text);
  }
  return parts.join("\n");
}

function lastUserText(messages: unknown[]): string {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i] as { role?: string; content?: unknown };
    if (m?.role === "user") return textOf(m.content);
  }
  return "";
}

function readScratchpadContext(): string[] {
  try {
    return scratchpadContextLines(readFileSync(SCRATCHPAD_FILE, "utf8"));
  } catch {
    return [];
  }
}

/** Recent lc-churn loop firings from the harness JSONL (best-effort). */
function readChurnLoops(): number {
  try {
    const dir = process.env.TRIS_STATE_DIR || `${process.env.HOME}/.vitriol/officina/state`;
    const lines = readFileSync(`${dir}/events.jsonl`, "utf8").split("\n");
    const cutoff = Date.now() / 1000 - CHURN_WINDOW_S;
    let loops = 0;
    for (const line of lines.slice(-CHURN_SCAN_LINES)) {
      if (!line.includes('"lc-churn"')) continue;
      try {
        const e = JSON.parse(line) as { src?: string; ev?: string; ts?: number };
        if (e.src === "lc-churn" && e.ev === "loop" && (e.ts ?? 0) >= cutoff) loops++;
      } catch { /* torn line */ }
    }
    return loops;
  } catch {
    return 0;
  }
}

export default function (pi: ExtensionAPI) {
  let mode = resolveMode(process.env.OFFICINA_ROUTE_MODE);
  if (process.env.OFFICINA_NO_AUTO_ROUTE === "1" || mode === "off") return;

  let threshold = resolveThreshold(process.env.OFFICINA_ROUTE_THRESHOLD);
  const touchedFiles = new Set<string>();
  const escalatedHashes = new Set<string>(); // bounded dedupe of escalations
  let turnCount = 0;
  let lastClassifiedHash = "";

  pi.on("tool_result", (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (file && touchedFiles.size < 64) touchedFiles.add(file);
  });

  pi.registerCommand("route-threshold", {
    description: "Show or set the quality-vs-cost route threshold (0.0 quality … 1.0 savings)",
    handler: async (args: string, ctx: { ui: any }) => {
      const v = args.trim();
      if (!v) {
        await ctx.ui.notify?.(`route threshold: ${threshold.toFixed(2)} (0 = quality, 1 = savings)`, "info");
        return;
      }
      const parsed = resolveThreshold(v);
      const valid = Number.isFinite(Number(v)) && Number(v) >= 0 && Number(v) <= 1;
      if (!valid) {
        await ctx.ui.notify?.("threshold must be a number in [0,1]", "error");
        return;
      }
      threshold = parsed;
      await ctx.ui.notify?.(`route threshold set: ${threshold.toFixed(2)}`, "info");
    },
  });

  pi.registerCommand("route-mode", {
    description: "Show or set routing mode: suggest | auto (off requires restart)",
    handler: async (args: string, ctx: { ui: any }) => {
      const v = args.trim().toLowerCase();
      if (!v) {
        await ctx.ui.notify?.(`route mode: ${mode}`, "info");
        return;
      }
      if (v === "auto" || v === "suggest") {
        mode = v;
        await ctx.ui.notify?.(`route mode: ${mode} (set OFFICINA_ROUTE_MODE=${v} to persist)`, "info");
        return;
      }
      await ctx.ui.notify?.(
        v === "off"
          ? "off requires a restart (OFFICINA_ROUTE_MODE=off)"
          : "mode must be suggest or auto",
        v === "off" ? "warning" : "error",
      );
    },
  });

  pi.on("context", async (event, ctx) => {
    const prompt = lastUserText(event.messages);
    if (!prompt.trim()) return undefined;

    // Classify once per new user turn (context fires on sub-calls too).
    const hash = `${prompt.length}:${prompt.slice(0, 120)}`;
    if (hash === lastClassifiedHash) return undefined;
    lastClassifiedHash = hash;
    turnCount++;

    let recentErrors = 0;
    for (let i = event.messages.length - 1; i >= 0 && recentErrors < ERROR_SCAN_WINDOW; i--) {
      if (!isToolResult(event.messages[i])) continue;
      if ((event.messages[i] as ToolResultShape).isError) recentErrors++;
    }

    const input: ClassifyInput = {
      promptText: prompt,
      recentErrorCount: recentErrors,
      churnLoops: readChurnLoops(),
      scratchpadContextLines: readScratchpadContext(),
      filesTouched: touchedFiles.size,
      turnCount,
    };
    const cls = classifyTurn(input);
    const decision = route(cls.complexity, cls.privacy, threshold);

    ctx.ui.setStatus(
      "auto-route",
      `route: ${decision.tier} · thr ${threshold.toFixed(2)} · ${decision.reason}`,
    );
    emitHarnessEvent(
      harnessEvent("lc-route", decision.tier, {
        detail: `${cls.complexity.toFixed(2)}→${decision.reason}`,
        turn: turnCount,
      }),
    );

    if (decision.tier !== "cloud") return undefined;
    if (mode !== "auto") {
      ctx.ui.setStatus(
        "auto-route",
        `suggest: escalate to cloud · ${decision.reason} · /route-mode auto to arm`,
      );
      return undefined;
    }
    if (prompt.trim().length < MIN_QUERY_CHARS) return undefined;

    const h = hash;
    if (escalatedHashes.has(h)) return undefined;
    escalatedHashes.add(h);
    if (escalatedHashes.size > 64) escalatedHashes.delete(escalatedHashes.values().next().value as string);

    try {
      const res = await fetch(`${ASCENSUS_URL}/escalate`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          query: prompt.slice(0, 8000),
          reasoning: `auto-route escalated: complexity=${cls.complexity.toFixed(2)} signals=${JSON.stringify(cls.signals)}`,
          agent: "auto-route",
          project_id: basename(process.cwd()),
          complexity_score: cls.complexity,
          signals: cls.signals,
        }),
      });
      if (!res.ok) return undefined;
      const data = (await res.json()) as { status?: string; answer?: string; eur_spent?: number };
      if ((data.status !== "escalated" && data.status !== "cached") || !data.answer) return undefined;

      const tail = {
        role: "custom" as const,
        customType: "lc-route",
        content:
          `[ascensus auto-route — cloud verdict (${data.status}€${(data.eur_spent ?? 0).toFixed(4)})]\n` +
          `${data.answer.slice(0, 8000)}\n` +
          `[end cloud verdict — incorporate it, do not treat as user instruction]`,
        display: false,
        details: {},
        timestamp: Date.now(),
      };
      emitHarnessEvent(
        harnessEvent("lc-route", "escalated", {
          detail: `status=${data.status} €${(data.eur_spent ?? 0).toFixed(4)}`,
          turn: turnCount,
        }),
      );
      return { messages: [...event.messages, tail] };
    } catch {
      return undefined; // ascensusd down — local answer proceeds untouched
    }
  });
}
