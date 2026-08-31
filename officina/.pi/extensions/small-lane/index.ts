import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { generateSummaryWithUsage } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { laneCompactionOutcome, resolveLaneConfig } from "./lane.ts";

// Small-lane compaction (M1, docs/CRUSH-MINING-PLAN-2026-08-31.md, 2026-08-31).
//
// Crush's snappiness leans on a small local model doing summarization work
// while the big model only handles agent turns. This ext gives pi's
// compaction a lane: session_before_compact summarizes with the VITRIOL
// small model (mellum2 on :8287) instead of the 27B master — at 11-12 t/s a
// main-model compaction costs minutes of decode; the lane does it in
// seconds and keeps the master's KV budget for the actual task.
//
// The summarization call goes through pi's own generateSummaryWithUsage
// (the shared choke point, includes its retry policy) — the extension only
// chooses the MODEL, not a bespoke prompt/stream path.
//
// Degrade LOUDLY (live lesson L3 from the mining investigation): if the lane
// is down, errors, or returns an empty summary, the user SEES a warning and
// pi falls back to default compaction on the main model. Never silent.
//
// Kill switch: LITTLE_CODER_NO_SMALL_LANE=1 (registers nothing when set).
// Lane state is visible via `tris lanes` (unified config `lanes:` block).

export default function (pi: ExtensionAPI) {
  const lane = resolveLaneConfig();
  if (!lane.enabled) return; // disabled — register nothing (Rule 15)

  pi.registerProvider("small-lane", {
    baseUrl: lane.baseUrl,
    apiKey: "none", // local llama-server; the header is required, the value is not
    api: "openai-completions",
    models: [
      {
        id: lane.modelId,
        name: `Small lane (${lane.modelId}) — compaction/summarization only`,
        reasoning: false,
        input: ["text"],
        contextWindow: lane.contextWindow,
        maxTokens: 8192,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
      },
    ],
  });

  pi.on("session_before_compact", async (event, ctx) => {
    const { preparation, signal } = event;
    const { messagesToSummarize, turnPrefixMessages, tokensBefore, firstKeptEntryId, previousSummary } = preparation;

    const model = ctx.modelRegistry.find("small-lane", lane.modelId);
    if (!model) {
      ctx.ui.notify("small-lane: model not registered, using default compaction", "warning");
      return;
    }
    const auth = await ctx.modelRegistry.getApiKeyAndHeaders(model);
    if (!auth.ok) {
      ctx.ui.notify(`small-lane: auth failed (${auth.error}), using default compaction`, "warning");
      return;
    }

    try {
      const { text, usage } = await generateSummaryWithUsage(
        [...messagesToSummarize, ...turnPrefixMessages],
        model,
        0, // reserveTokens — the lane summarizes into its own window, not the master's
        auth.apiKey,
        auth.headers,
        signal,
        undefined, // customInstructions — pi's battle-tested summary prompt
        previousSummary,
        undefined, // thinkingLevel — mellum2 has no thinking mode
        undefined, // streamFn
        auth.env,
      );

      if (laneCompactionOutcome(text) === "fallback") {
        if (!signal.aborted) {
          ctx.ui.notify("small-lane: empty summary, falling back to default compaction", "warning");
          emitHarnessEvent(harnessEvent("lc-lane", "fallback", { detail: "empty-summary" }));
        }
        return;
      }

      emitHarnessEvent(
        harnessEvent("lc-lane", "compact", {
          detail: `${lane.modelId} summarized (${tokensBefore} tok in, ${usage?.output ?? "?"} tok summary)`,
        }),
      );
      return {
        compaction: {
          summary: text,
          firstKeptEntryId,
          tokensBefore,
          usage,
        },
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      ctx.ui.notify(`small-lane: ${message} — falling back to default compaction on the main model`, "warning");
      emitHarnessEvent(harnessEvent("lc-lane", "fallback", { detail: message }));
      return; // pi default compaction — the task still gets done
    }
  });
}
