import { setTimeout as delay } from "node:timers/promises";
import { getApiProvider } from "@earendil-works/pi-ai/compat";
import type { ProviderConfig } from "@earendil-works/pi-coding-agent";

const CAPACITY_CODES = new Set(["SEMAPHORE_TIMEOUT", "SEMAPHORE_QUEUE_FULL"]);
const WAIT_BUDGET_MS = 120_000;

/** Recognize gateway admission errors, never natural-language guesses about quota. */
function hasCapacityCode(text: string): boolean {
  const records = [text, ...text.split("\n").filter((line) => line.startsWith("data: ")).map((line) => line.slice(6))];
  return records.some((record) => {
    try {
      const code = JSON.parse(record)?.error?.code;
      return typeof code === "string" && CAPACITY_CODES.has(code.toUpperCase());
    } catch { return false; }
  });
}

/** Bound inspection of an error body; unknown, oversized or stalled bodies retain normal handling. */
async function isCapacityResponse(response: Response): Promise<boolean> {
  const reader = response.clone().body?.getReader();
  if (!reader) return false;
  let expired = false;
  const timer = setTimeout(() => { expired = true; void reader.cancel().catch(() => {}); }, 1000);
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const part = await reader.read();
      if (part.done) break;
      size += part.value.byteLength;
      if (size > 16_384) return false;
      chunks.push(part.value);
    }
    return !expired && hasCapacityCode(Buffer.concat(chunks).toString("utf8"));
  } catch { return false; }
  finally {
    clearTimeout(timer);
    // A tee's cancellation promise can wait for its other branch: never await it here.
    void reader.cancel().catch(() => {});
  }
}

/** Respect Retry-After without extending the finite admission budget. */
function retryDelay(response: Response): number {
  const hint = response.headers.get("retry-after");
  if (!hint) return 1000;
  const ms = /^\d+(\.\d+)?$/.test(hint.trim()) ? Number(hint) * 1000 : Date.parse(hint) - Date.now();
  return Number.isFinite(ms) ? Math.max(1000, ms) : 1000;
}

/** Finish rejected admission and wait without holding a response body or ignoring cancellation. */
async function waitForCapacity(response: Response, deadline: number, signal?: AbortSignal | null): Promise<boolean> {
  if (!await isCapacityResponse(response)) return false;
  const waitMs = retryDelay(response);
  if (Date.now() + waitMs >= deadline) return false;
  void response.body?.cancel().catch(() => {});
  await delay(waitMs, undefined, { signal: signal ?? undefined });
  signal?.throwIfAborted();
  if (Date.now() >= deadline) throw new Error("Caduceus capacity wait budget expired");
  return true;
}

/** Rejoin Caduceus's existing queue only after an explicit pre-execution capacity rejection. */
export function capacityAwareFetch(fetcher: typeof fetch): typeof fetch {
  let retries = 0;
  const deadline = Date.now() + WAIT_BUDGET_MS;
  return async (input, init) => {
    while (true) {
      const response = await fetcher(input, init);
      if (response.status !== 429 || retries >= 2 || typeof init?.body !== "string") return response;
      const signal = init?.signal ?? (input instanceof Request ? input.signal : undefined);
      if (!await waitForCapacity(response, deadline, signal)) return response;
      retries++;
    }
  };
}

/** Keep two sessions on the same cloud model while the gateway can alternate accepted requests. */
export const streamCloudWithCapacity: NonNullable<ProviderConfig["streamSimple"]> = (model, context, options) => {
  const api = getApiProvider(model.api);
  if (!api) throw new Error(`No API provider registered for api: ${model.api}`);
  if (model.provider !== "caduceus" || model.api !== "openai-completions" || process.env.OFFICINA_NO_CAPACITY_WAIT === "1") {
    return api.streamSimple(model, context, options);
  }
  return api.streamSimple(model, context, { ...options, fetch: capacityAwareFetch(options?.fetch ?? globalThis.fetch) });
};
