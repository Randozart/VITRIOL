import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { generateSummaryWithUsage } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import {
  estimateMessagesTokens,
  orderSummarizers,
  laneCompactionOutcome,
  resolveLaneConfig,
} from "./lane.ts";

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
  const smartFallback = process.env.OFFICINA_NO_SMART_COMPACTION !== "1";
  if (!lane.enabled && !smartFallback) return; // nothing to do (Rule 15)

  if (lane.enabled) {
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
  }

  pi.on("session_before_compact", async (event, ctx) => {
    const { preparation, signal } = event;
    const { messagesToSummarize, turnPrefixMessages, tokensBefore, firstKeptEntryId, previousSummary } = preparation;

    const messages = [...messagesToSummarize, ...turnPrefixMessages];
    // Fit-aware summarizer (2026-11-14): compaction hands the WHOLE
    // transcript to the summarizer. The configured small lane (when enabled)
    // is tiny and often unreachable; pi's default uses the current model,
    // which may be a 128k cloud lane while the transcript is 180k+. Choose a
    // model that actually fits the request.
    const estimate = estimateMessagesTokens(messages);
    const candidates = ctx.modelRegistry
      .getAll()
      .filter(
        (m) =>
          m.provider !== "small-lane" &&
          ctx.modelRegistry.hasConfiguredAuth(m) &&
          Number(m.contextWindow ?? 0) > 0,
      )
      .map((m) => ({ provider: m.provider, id: m.id, contextWindow: Number(m.contextWindow ?? 0) }));
    const order = orderSummarizers(candidates, estimate);

    // Candidate chain: the dedicated lane first (when enabled), then every
    // fitting model by descending window, cloud before local on ties. A 429
    // or quota 403 on one summarizer must not end compaction — try the next
    // one, down to the local stone.
    const maxTries = Math.max(1, Number(process.env.OFFICINA_COMPACTION_MAX_TRIES) || 6);
    const full: Array<{ provider: string; id: string }> = [];
    if (lane.enabled) full.push({ provider: "small-lane", id: lane.modelId });
    for (const c of order) full.push({ provider: c.provider, id: c.id });
    // Bound the attempts; always keep the local stone in the chain — it is
    // the one summarizer whose window and availability we control.
    const localCand = order.find((c) => c.provider === "llamacpp");
    const chain = full.slice(0, maxTries);
    if (localCand && !chain.some((c) => c.provider === "llamacpp")) {
      chain.push({ provider: localCand.provider, id: localCand.id });
    }

    if (chain.length === 0) {
      ctx.ui.notify(
        `compaction: no model fits ${estimate} tokens — using default compaction`,
        "warning",
      );
      return;
    }

    let lastError = "";
    for (const cand of chain) {
      const model = ctx.modelRegistry.find(cand.provider, cand.id);
      if (!model) continue;
      const auth = await ctx.modelRegistry.getApiKeyAndHeaders(model);
      if (!auth.ok) {
        lastError = `auth ${auth.error}`;
        continue;
      }
      const headers = auth.headers
        ? Object.entries(auth.headers).reduce<Record<string, string>>((acc, [key, value]) => {
            if (value !== null) acc[key] = value;
            return acc;
          }, {})
        : undefined;
      try {
        const { text, usage } = await generateSummaryWithUsage(
          messages,
          model,
          0, // reserveTokens — the summarizer owns its own window
          auth.apiKey,
          headers,
          signal,
          undefined, // customInstructions — pi's battle-tested summary prompt
          previousSummary,
          undefined, // thinkingLevel
          undefined, // streamFn
          auth.env,
        );
        if (laneCompactionOutcome(text) === "fallback") {
          lastError = "empty summary";
          continue;
        }
        if (model.provider !== "small-lane") {
          ctx.ui.notify(
            `compaction: summarized by ${model.provider}/${model.id} (est. ${estimate} tok)`,
            "info",
          );
        }
        emitHarnessEvent(
          harnessEvent("lc-lane", "compact", {
            detail: `${model.provider}/${model.id} summarized (${tokensBefore} tok in, ${usage?.output ?? "?"} tok summary)`,
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
        lastError = error instanceof Error ? error.message : String(error);
        if (signal.aborted) return;
        // Try the next fitting summarizer.
      }
    }

    if (!signal.aborted) {
      ctx.ui.notify(
        `compaction: every summarizer failed (${lastError}) — falling back to default compaction`,
        "warning",
      );
      emitHarnessEvent(harnessEvent("lc-lane", "fallback", { detail: lastError }));
    }
    return; // pi default compaction — the task still gets done
  });
}
