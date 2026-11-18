import { appendFileSync } from "node:fs";
import { getApiProvider } from "@earendil-works/pi-ai/compat";
import type { ProviderConfig } from "@earendil-works/pi-coding-agent";
import { observeLocalTransport } from "./transport-diagnostics.ts";
import { localCacheOwner } from "./owner.ts";
import { attachStructuredOutput } from "../_shared/structured-output.ts";

/** Keep healthy prefill alive using this request's native SSE progress, not global activity. */
export const streamLocalWithProgress: NonNullable<
  ProviderConfig["streamSimple"]
> = (model, context, options) => {
  const api = getApiProvider(model.api);
  if (!api) throw new Error(`No API provider registered for api: ${model.api}`);
  if (model.provider !== "llamacpp" || model.api !== "openai-completions") {
    return api.streamSimple(model, context, options);
  }
  observeLocalTransport(model.baseUrl);
  const progress = process.env.OFFICINA_NO_PREFILL_PROGRESS !== "1";
  const owner = localCacheOwner(options?.sessionId);
  if (!progress && !owner) return api.streamSimple(model, context, options);
  return api.streamSimple(model, context, {
    ...options,
    onPayload: async (payload, requestModel) => {
      if (process.env.OFFICINA_TRACE_MAXTOKENS) {
        try {
          appendFileSync(
            process.env.OFFICINA_TRACE_MAXTOKENS,
            `${JSON.stringify({ id: model.id, ctxWindow: model.contextWindow, maxTokens: model.maxTokens, payloadMax: (payload as Record<string, unknown>)?.max_tokens })}\n`,
          );
        } catch {
          /* tracing must never break a stream */
        }
      }
      const request = {
        ...(payload as Record<string, unknown>),
        ...(progress ? { return_progress: true } : {}),
        ...(owner
          ? {
              cache_owner_session: owner.session,
              cache_owner_branch: owner.branch,
            }
          : {}),
      };
      // Structured lanes (2026-11-18): a prompt that asks for JSON via
      // `[[officina-json <schema>]]` gets a real grammar. Measured: nested schemas
      // went from INVALID 3/3 and 533 chars to valid 3/3 at 310 chars, while a
      // simple flat schema showed no gain — so the prompt decides, not the provider.
      const structured = attachStructuredOutput(request);
      // Hooks see the progress option and retain their existing replacement contract.
      return (
        (await options?.onPayload?.(structured, requestModel)) ?? structured
      );
    },
  });
};
