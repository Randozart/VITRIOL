import { setTimeout as delay } from "node:timers/promises";
import { getApiProvider } from "@earendil-works/pi-ai/compat";
import type { ProviderConfig } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";

const CAPACITY_CODES = new Set(["SEMAPHORE_TIMEOUT", "SEMAPHORE_QUEUE_FULL"]);
const TRANSIENT_STATUSES = new Set([408, 500, 502, 503, 504, 520, 524, 529]);
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
async function readErrorText(response: Response): Promise<string | undefined> {
  const reader = response.clone().body?.getReader();
  if (!reader) return undefined;
  let expired = false;
  const timer = setTimeout(() => { expired = true; void reader.cancel().catch(() => {}); }, 1000);
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const part = await reader.read();
      if (part.done) break;
      size += part.value.byteLength;
      if (size > 16_384) return undefined;
      chunks.push(part.value);
    }
    return expired ? undefined : Buffer.concat(chunks).toString("utf8");
  } catch { return undefined; }
  finally {
    clearTimeout(timer);
    // A tee's cancellation promise can wait for its other branch: never await it here.
    void reader.cancel().catch(() => {});
  }
}

/** Preserve retry classification and cooldown hints when the SDK retains only error.message. */
async function normalizeModelCooldown(response: Response): Promise<Response> {
  if (response.status !== 429 || response.headers.get("x-omniroute-local-cooldown") !== "model") return response;
  const text = await readErrorText(response);
  try {
    const body = JSON.parse(text ?? "");
    if (body?.error?.code !== "model_cooldown" || typeof body.error.message !== "string") return response;
    const hints = { reset_seconds: body.error.reset_seconds, retry_after: body.error.retry_after };
    body.error.message = `429 model_cooldown: ${body.error.message}\n${JSON.stringify(hints)}`;
    const headers = new Headers(response.headers);
    headers.delete("content-length");
    headers.delete("content-encoding");
    headers.set("content-type", "application/json");
    void response.body?.cancel().catch(() => {});
    return new Response(JSON.stringify(body), { status: response.status, statusText: response.statusText, headers });
  } catch { return response; }
}

/** Respect Retry-After without extending the finite admission budget. */
function retryDelay(response: Response, retry: number): number {
  const hint = response.headers.get("retry-after");
  if (!hint) return 1000 * 2 ** retry;
  const ms = /^\d+(\.\d+)?$/.test(hint.trim()) ? Number(hint) * 1000 : Date.parse(hint) - Date.now();
  return Number.isFinite(ms) ? Math.max(1000, ms) : 1000;
}

/** Keep temporary HTTP rejections internal while preserving permanent/unknown errors. */
async function isRecoverableResponse(response: Response): Promise<boolean> {
  if (response.headers.get("x-should-retry")?.trim().toLowerCase() === "false") return false;
  if (TRANSIENT_STATUSES.has(response.status)) return true;
  if (response.status !== 429) return false;
  const text = await readErrorText(response);
  return text !== undefined && hasCapacityCode(text);
}

/** Finish rejected admission and wait without holding a response body or ignoring cancellation. */
async function waitForRecovery(response: Response, deadline: number, retry: number, signal?: AbortSignal | null): Promise<boolean> {
  if (!await isRecoverableResponse(response)) return false;
  const waitMs = retryDelay(response, retry);
  if (Date.now() + waitMs >= deadline) return false;
  void response.body?.cancel().catch(() => {});
  emitHarnessEvent(harnessEvent("lc-route", "gateway-recovery-wait", {
    detail: `HTTP ${response.status}; retry ${retry + 1}/2; wait ${waitMs}ms`,
  }));
  await delay(waitMs, undefined, { signal: signal ?? undefined });
  signal?.throwIfAborted();
  if (Date.now() >= deadline) throw new Error("Caduceus recovery wait budget expired");
  return true;
}

/** Recover rejected HTTP attempts before they escape into the session; never replay an accepted stream. */
export function recoveringGatewayFetch(fetcher: typeof fetch): typeof fetch {
  let retries = 0;
  const deadline = Date.now() + WAIT_BUDGET_MS;
  return async (input, init) => {
    while (true) {
      const response = await normalizeModelCooldown(await fetcher(input, init));
      if (response.status < 400 || retries >= 2 || typeof init?.body !== "string") return response;
      const signal = init?.signal ?? (input instanceof Request ? input.signal : undefined);
      if (!await waitForRecovery(response, deadline, retries, signal)) return response;
      retries++;
    }
  };
}

/** Keep recoverable gateway rejections inside the provider, preserving accepted streams and session state. */
export const streamCloudWithRecovery: NonNullable<ProviderConfig["streamSimple"]> = (model, context, options) => {
  const api = getApiProvider(model.api);
  if (!api) throw new Error(`No API provider registered for api: ${model.api}`);
  if (model.provider !== "caduceus" || model.api !== "openai-completions" || process.env.OFFICINA_NO_CAPACITY_WAIT === "1") {
    return api.streamSimple(model, context, options);
  }
  return api.streamSimple(model, context, { ...options, fetch: recoveringGatewayFetch(options?.fetch ?? globalThis.fetch) });
};
