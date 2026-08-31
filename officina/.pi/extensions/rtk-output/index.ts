import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { mkdirSync, writeFileSync } from "node:fs";
import { harnessIntervention } from "../_shared/intervention.ts";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { isRtkTarget, rawPathFor, renderSummary, rtkConfig, summarize } from "./rtk.ts";

// rtk-output — RTK-style command-output filtering at ENTRY (OmniRoute §4.1,
// REPORT-02 step 8). Test/build/install output is reduced to exit status +
// error lines + verbatim tail BEFORE it enters context; the full raw payload
// is written to .pi/rtk/<toolCallId>.log and referenced by path so it is
// always recoverable. Target: 60-90% reduction on typical test/build output.
//
// Layering (why this and not more clearing): rtk filters at ENTRY (bloat never
// arrives), tool-result-clearer evicts AFTER consumption, read-guard covers
// file reads. All three are independent, kill-switched stages (Rule 8/15).
//
// Kill switch: TRIS_NO_RTK_OUTPUT=1 (Rule 15).
// Tuning: TRIS_RTK_THRESHOLD (min chars to act, default 800), TRIS_RTK_TAIL,
// TRIS_RTK_ERRCAP, TRIS_RTK_DIR. Unified config: context_pipeline.clear.rtk_output.

export default function (pi: ExtensionAPI) {
  const cfg = rtkConfig();
  if (!cfg.enabled) return;

  pi.on("tool_result", async (event, ctx) => {
    const toolName = String((event as { toolName?: string }).toolName ?? "").toLowerCase();
    if (toolName !== "bash") return;
    const input = (event as { input?: Record<string, unknown> }).input ?? {};
    const command = String(input.command ?? "");
    if (!command || !isRtkTarget(command)) return;

    const content = ((event as { content?: { type: string; text?: string }[] }).content ?? []);
    if (content.some((c) => c.type !== "text")) return; // images pass through
    const text = content.map((c) => c.text ?? "").join("");
    if (text.length < cfg.thresholdChars) return; // small outputs are already knowledge

    const summary = summarize(command, text, cfg);
    const rawPath = rawPathFor(cfg.rawDir, String((event as { toolCallId?: string }).toolCallId ?? "unknown"));
    try {
      mkdirSync(cfg.rawDir, { recursive: true });
      writeFileSync(rawPath, `$ ${command}\n\n${text}`);
    } catch {
      return; // disk trouble must never eat the real result — pass raw through
    }

    harnessIntervention(
      ctx,
      `bash output reduced at entry (${Math.round(summary.savedChars / 1024)}K chars dropped, raw kept at ${rawPath}).`,
    );
    emitHarnessEvent(harnessEvent("lc-rtk", "reduced", { freed_tokens: Math.round(summary.savedChars / 4), detail: command.slice(0, 60) }));
    return { content: [{ type: "text" as const, text: renderSummary(summary, rawPath) }] };
  });
}
