import { getApiProvider } from "@earendil-works/pi-ai/compat";
import type { ProviderConfig } from "@earendil-works/pi-coding-agent";
import { observeLocalTransport } from "./transport-diagnostics.ts";

/** Keep healthy prefill alive using this request's native SSE progress, not global activity. */
export const streamLocalWithProgress: NonNullable<ProviderConfig["streamSimple"]> = (model, context, options) => {
  const api = getApiProvider(model.api);
  if (!api) throw new Error(`No API provider registered for api: ${model.api}`);
  if (model.provider !== "llamacpp" || model.api !== "openai-completions") {
    return api.streamSimple(model, context, options);
  }
  observeLocalTransport(model.baseUrl);
  if (process.env.OFFICINA_NO_PREFILL_PROGRESS === "1") return api.streamSimple(model, context, options);
  return api.streamSimple(model, context, {
    ...options,
    onPayload: async (payload, requestModel) => {
      const request = { ...(payload as Record<string, unknown>), return_progress: true };
      // Hooks see the progress option and retain their existing replacement contract.
      return await options?.onPayload?.(request, requestModel) ?? request;
    },
  });
};
