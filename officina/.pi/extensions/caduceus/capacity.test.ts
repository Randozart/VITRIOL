import { afterEach, describe, expect, it, vi } from "vitest";
import { normalizeContext } from "@earendil-works/pi-ai";
// SDK loading belongs to fixture setup, outside transport assertion deadlines.
import "@earendil-works/pi-ai/api/openai-completions";
import { isRetryableAssistantError } from "@earendil-works/pi-ai/utils/retry";
import { parseCooldownFromError } from "../_shared/gateway-health.ts";
import { recoveringGatewayFetch, streamCloudWithRecovery } from "./capacity.ts";

const recoveryEvents = vi.hoisted(() => [] as Array<{ detail?: string }>);
vi.mock("../_shared/events.ts", () => ({
  emitHarnessEvent: (event: { detail?: string }) => recoveryEvents.push(event),
  harnessEvent: (_source: unknown, _event: unknown, fields: unknown) => fields,
}));

const capacity = (code = "SEMAPHORE_TIMEOUT", headers = {}) => new Response(JSON.stringify({ error: { code } }), { status: 429, headers });
const request = { method: "POST", body: JSON.stringify({ model: "shared", messages: ["private context"] }) };
afterEach(() => { vi.useRealTimers(); vi.unstubAllEnvs(); recoveryEvents.length = 0; });

describe("gateway capacity admission", () => {
  it.each([408, 500, 502, 503, 504, 520, 524, 529])("recovers transient HTTP %s before returning a response", async (status) => {
    vi.useFakeTimers();
    const final = new Response("accepted");
    const fetcher = vi.fn().mockResolvedValueOnce(new Response("temporarily unavailable", { status })).mockResolvedValue(final);
    const pending = recoveringGatewayFetch(fetcher)("http://gateway", request);
    await vi.advanceTimersByTimeAsync(1000);
    expect(await pending).toBe(final);
    expect(fetcher.mock.calls.map((call) => call[1])).toEqual([request, request]);
  });

  it.each([429, 503])("honors explicit no-retry on HTTP %s", async (status) => {
    const response = new Response(JSON.stringify({ error: { code: "SEMAPHORE_TIMEOUT" } }), { status, headers: { "x-should-retry": "false" } });
    const fetcher = vi.fn().mockResolvedValue(response);
    expect(await recoveringGatewayFetch(fetcher)("http://gateway", request)).toBe(response);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("shares the same recovery budget between capacity and transient errors", async () => {
    const final = new Response("still unavailable", { status: 503 });
    const fetcher = vi.fn().mockResolvedValueOnce(capacity()).mockResolvedValue(final);
    expect(await recoveringGatewayFetch(fetcher)("http://gateway", request)).toBe(final);
    expect(fetcher).toHaveBeenCalledTimes(3);
  });
  it.each(["SEMAPHORE_TIMEOUT", "SEMAPHORE_QUEUE_FULL", "semaphore_timeout"])("retries %s with identical body and options", async (code) => {
    vi.useFakeTimers();
    const final = new Response("accepted");
    const fetcher = vi.fn().mockResolvedValueOnce(capacity(code)).mockResolvedValue(final);
    const result = recoveringGatewayFetch(fetcher)("http://gateway/v1/chat/completions", request);
    await vi.advanceTimersByTimeAsync(1000);
    expect(await result).toBe(final);
    expect(fetcher.mock.calls).toEqual([["http://gateway/v1/chat/completions", request], ["http://gateway/v1/chat/completions", request]]);
  });

  it.each([400, 401, 403, 404, 409, 422, 501])( "passes status %s through untouched", async (status) => {
    const response = new Response("failure", { status });
    const fetcher = vi.fn().mockResolvedValue(response);
    expect(await recoveringGatewayFetch(fetcher)("http://gateway", request)).toBe(response);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(await response.text()).toBe("failure");
  });

  it.each([
    '{"error":{"code":"rate_limit_exceeded","message":"quota exhausted"}}',
    '{"error":{"message":"SEMAPHORE_TIMEOUT"}}',
    '{"code":"SEMAPHORE_TIMEOUT"}',
    "too many concurrent requests", "malformed", "x".repeat(17_000),
  ])("preserves unknown/quota 429 body %#", async (body) => {
    const response = new Response(body, { status: 429 });
    const fetcher = vi.fn().mockResolvedValue(response);
    expect(await recoveringGatewayFetch(fetcher)("http://gateway", request)).toBe(response);
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(await response.text()).toBe(body);
  });

  it("recognizes the gateway's SSE error envelope", async () => {
    vi.useFakeTimers();
    const fetcher = vi.fn().mockResolvedValueOnce(new Response('data: {"error":{"code":"SEMAPHORE_QUEUE_FULL"}}\n\ndata: [DONE]\n\n', { status: 429 })).mockResolvedValue(new Response("OK"));
    const pending = recoveringGatewayFetch(fetcher)("http://gateway", request);
    await vi.advanceTimersByTimeAsync(1000);
    expect((await pending).status).toBe(200);
  });

  it("shares a two-retry budget across SDK attempts and exposes the final error", async () => {
    vi.useFakeTimers();
    const fetcher = vi.fn().mockImplementation(async () => capacity());
    const wrapped = recoveringGatewayFetch(fetcher);
    const pending = wrapped("http://gateway", request);
    await vi.advanceTimersByTimeAsync(3000);
    expect((await pending).status).toBe(429);
    expect(fetcher).toHaveBeenCalledTimes(3);
    expect((await wrapped("http://gateway", request)).status).toBe(429);
    expect(fetcher).toHaveBeenCalledTimes(4);
  });

  it.each(["seconds", "date"])("honors Retry-After %s", async (format) => {
    vi.useFakeTimers();
    const hint = format === "seconds" ? "3" : new Date(Date.now() + 3000).toUTCString();
    const fetcher = vi.fn().mockResolvedValueOnce(capacity("SEMAPHORE_TIMEOUT", { "Retry-After": hint })).mockResolvedValue(new Response("OK"));
    const pending = recoveringGatewayFetch(fetcher)("http://gateway", request);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2000);
    expect((await pending).status).toBe(200);
  });

  it("does not shorten a Retry-After beyond its admission budget", async () => {
    const response = capacity("SEMAPHORE_TIMEOUT", { "Retry-After": "121" });
    const fetcher = vi.fn().mockResolvedValue(response);
    expect(await recoveringGatewayFetch(fetcher)("http://gateway", request)).toBe(response);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("cancels during the wait without another request", async () => {
    vi.useFakeTimers();
    const abort = new AbortController();
    const fetcher = vi.fn().mockResolvedValue(capacity());
    const pending = recoveringGatewayFetch(fetcher)("http://gateway", { ...request, signal: abort.signal });
    const rejected = expect(pending).rejects.toMatchObject({ name: "AbortError" });
    await vi.advanceTimersByTimeAsync(100);
    abort.abort();
    await rejected;
    await vi.advanceTimersByTimeAsync(5000);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("leaves stalled error bodies to the SDK after bounded inspection", async () => {
    vi.useFakeTimers();
    const response = new Response(new ReadableStream(), { status: 429 });
    const fetcher = vi.fn().mockResolvedValue(response);
    const pending = recoveringGatewayFetch(fetcher)("http://gateway", request);
    await vi.advanceTimersByTimeAsync(1000);
    expect(await pending).toBe(response);
    expect(fetcher).toHaveBeenCalledTimes(1);
    void response.body?.cancel();
  });

  it("does not replay a one-shot request body or retry a network failure", async () => {
    const response = capacity();
    const fetcher = vi.fn().mockResolvedValueOnce(response).mockRejectedValueOnce(new Error("network failed"));
    const wrapped = recoveringGatewayFetch(fetcher);
    expect(await wrapped("http://gateway", { method: "POST", body: new ReadableStream() })).toBe(response);
    await expect(wrapped("http://gateway", request)).rejects.toThrow("network failed");
    expect(fetcher).toHaveBeenCalledTimes(2);
  });
});

const model = { api: "openai-completions" as const, provider: "caduceus", id: "shared", name: "Shared cloud",
  baseUrl: "http://gateway.invalid/v1", reasoning: false, input: ["text"] as ["text"],
  contextWindow: 262144, maxTokens: 100, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } };
const context = (name: string) => normalizeContext({ messages: [{ role: "user", content: name, timestamp: 1 }] });
const answer = (name: string) => `data: ${JSON.stringify({ id: name, choices: [{ index: 0, delta: { role: "assistant", content: name }, finish_reason: null }] })}\n\ndata: ${JSON.stringify({ id: name, choices: [{ index: 0, delta: {}, finish_reason: "stop" }] })}\n\ndata: [DONE]\n\n`;

it("preserves the exact gateway model-cooldown error as retryable after SDK normalization", async () => {
  const message = "All credentials for model aion-labs/aion-3.0 are cooling down";
  const body = { error: { code: "model_cooldown", type: "rate_limit_error", message, reset_seconds: 120 } };
  const fetcher = vi.fn().mockImplementation(async () => new Response(JSON.stringify(body), {
    status: 429, headers: { "content-type": "application/json", "x-omniroute-local-cooldown": "model", "retry-after": "120" },
  }));
  const result = await streamCloudWithRecovery(model, context("hello"), { apiKey: "test", fetch: fetcher, maxRetries: 0 }).result();
  expect(result.errorMessage).toContain(message);
  expect(isRetryableAssistantError(result)).toBe(true);
  expect(parseCooldownFromError(result.errorMessage)).toBe(120_000);
  expect(isRetryableAssistantError({ ...result, errorMessage: message })).toBe(false);
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it.each(["insufficient_quota", "unknown_error"])("does not relabel %s even with a local cooldown header", async (code) => {
  const response = new Response(JSON.stringify({ error: { code, message: "unavailable" } }), {
    status: 429, headers: { "x-omniroute-local-cooldown": "model" },
  });
  expect(await recoveringGatewayFetch(vi.fn().mockResolvedValue(response))("http://gateway", request)).toBe(response);
  expect(await response.text()).not.toContain("429 model_cooldown");
});

it("keeps temporary failures out of SDK events while preserving the payload hook", async () => {
  vi.useFakeTimers();
  const fetcher = vi.fn().mockResolvedValueOnce(new Response("unavailable", { status: 503 }))
    .mockResolvedValue(new Response(answer("recovered"), { headers: { "content-type": "text/event-stream" } }));
  const onResponse = vi.fn();
  const onPayload = vi.fn((payload: unknown) => ({ ...payload as object, temperature: 0.1 }));
  const stream = streamCloudWithRecovery(model, context("hello"), { apiKey: "test", fetch: fetcher, maxRetries: 0, onResponse, onPayload });
  const events: string[] = [];
  const consume = (async () => { for await (const event of stream) events.push(event.type); })();
  await vi.advanceTimersByTimeAsync(1000);
  await consume;
  const result = await stream.result();
  expect(result.stopReason).toBe("stop");
  expect(events).not.toContain("error");
  expect(events).toContain("done");
  expect(onResponse).toHaveBeenCalledTimes(1);
  expect(onResponse.mock.calls[0][0].status).toBe(200);
  expect(onPayload).toHaveBeenCalledTimes(1);
  const bodies = fetcher.mock.calls.map((call) => call[1].body);
  expect(bodies).toHaveLength(2);
  expect(bodies[0]).toBe(bodies[1]);
  expect(JSON.parse(bodies[0]).temperature).toBe(0.1);
  expect(recoveryEvents).toEqual([{ detail: "HTTP 503; retry 1/2; wait 1000ms" }]);
  expect(JSON.stringify(recoveryEvents)).not.toContain("hello");
});

it("emits one final SDK error after transient recovery is exhausted", async () => {
  vi.useFakeTimers();
  const fetcher = vi.fn().mockImplementation(async () => new Response("unavailable", { status: 503 }));
  const stream = streamCloudWithRecovery(model, context("hello"), { apiKey: "test", fetch: fetcher, maxRetries: 0 });
  const events: string[] = [];
  const consume = (async () => { for await (const event of stream) events.push(event.type); })();
  await vi.advanceTimersByTimeAsync(3000);
  await consume;
  expect((await stream.result()).stopReason).toBe("error");
  expect(events.filter((event) => event === "error")).toHaveLength(1);
  expect(fetcher).toHaveBeenCalledTimes(3);
});

it.each([401, 429])("does not turn permanent HTTP %s into silent retries", async (status) => {
  const fetcher = vi.fn().mockImplementation(async () => new Response(JSON.stringify({ error: { code: "insufficient_quota" } }), { status }));
  const result = await streamCloudWithRecovery(model, context("hello"), { apiKey: "test", fetch: fetcher, maxRetries: 0 }).result();
  expect(result.stopReason).toBe("error");
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it("preserves the kill switch for transient HTTP recovery", async () => {
  vi.stubEnv("OFFICINA_NO_CAPACITY_WAIT", "1");
  const fetcher = vi.fn().mockImplementation(async () => new Response("unavailable", { status: 503 }));
  expect((await streamCloudWithRecovery(model, context("hello"), { apiKey: "test", fetch: fetcher, maxRetries: 0 }).result()).stopReason).toBe("error");
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it("never replays an accepted stream that fails while reading", async () => {
  const fetcher = vi.fn().mockImplementation(async () => new Response(new ReadableStream({ start(controller) {
    controller.enqueue(new TextEncoder().encode(answer("partial").split("\n\n")[0] + "\n\n"));
    setTimeout(() => controller.error(new Error("stream was interrupted")), 20);
  } }), { headers: { "content-type": "text/event-stream" } }));
  const stream = streamCloudWithRecovery(model, context("hello"), { apiKey: "test", fetch: fetcher, maxRetries: 0 });
  const events: string[] = [];
  for await (const event of stream) events.push(event.type);
  expect(events).toContain("text_delta");
  expect(events).toContain("error");
  expect((await stream.result()).stopReason).toBe("error");
  expect(fetcher).toHaveBeenCalledTimes(1);
});

it("lets two SDK sessions finish on the same model through a one-slot gateway", async () => {
  let occupied = false;
  let finishFirst: (() => void) | undefined;
  const accepted: string[] = [];
  let admitFirst!: () => void;
  const admitted = new Promise<void>((resolve) => { admitFirst = resolve; });
  const fetcher = vi.fn(async (_input: unknown, init?: RequestInit) => {
    const payload = JSON.parse(init?.body as string);
    expect(payload.model).toBe("shared");
    const name = payload.messages.at(-1).content;
    if (occupied) { finishFirst!(); return capacity(); }
    occupied = true;
    if (name === "first") admitFirst();
    accepted.push(name);
    const body = new ReadableStream({ start(controller) {
      controller.enqueue(new TextEncoder().encode(answer(name)));
      const finish = () => { occupied = false; controller.close(); };
      if (name === "first") finishFirst = finish;
      else finish();
    } });
    return new Response(body, { headers: { "Content-Type": "text/event-stream" } });
  });
  const statuses: number[] = [];
  const options = { apiKey: "test", fetch: fetcher as typeof fetch, maxRetries: 0,
    onResponse: (response: { status: number }) => { statuses.push(response.status); } };
  const first = streamCloudWithRecovery(model, context("first"), options).result();
  // Wait until the first request is admitted; no real network or provider calls.
  await Promise.race([admitted, first.then((result) => { throw new Error(`First session ended before admission: ${result.errorMessage}`); })]);
  const second = streamCloudWithRecovery(model, context("second"), options).result();
  const results = await Promise.all([first, second]);
  expect(results.map((result) => result.errorMessage)).toEqual([undefined, undefined]);
  expect(results.map((result) => result.stopReason)).toEqual(["stop", "stop"]);
  expect(results.map((result) => result.content)).toEqual([
    [{ type: "text", text: "first" }], [{ type: "text", text: "second" }],
  ]);
  expect(accepted).toEqual(["first", "second"]);
  expect(fetcher).toHaveBeenCalledTimes(3);
  expect(statuses).toEqual([200, 200]);
});
