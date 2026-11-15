import { afterEach, describe, expect, it, vi } from "vitest";
import { Duplex } from "node:stream";
import { createRequire } from "node:module";
import { channel } from "node:diagnostics_channel";
import type { Socket } from "node:net";
import { normalizeContext } from "@earendil-works/pi-ai";
// SDK loading belongs to fixture setup, outside transport assertion deadlines.
import "@earendil-works/pi-ai/api/openai-completions";
import { streamLocalWithProgress } from "./progress.ts";

const traces = vi.hoisted(() => [] as Array<{ detail?: string }>);
vi.mock("../_shared/events.ts", () => ({
  emitHarnessEvent: (event: { detail?: string }) => traces.push(event),
  harnessEvent: (_src: unknown, _event: unknown, fields: unknown) => fields,
}));

const require = createRequire(new URL("../../../node_modules/@earendil-works/pi-coding-agent/package.json", import.meta.url));
const { Client, fetch: httpFetch } = require("undici") as typeof import("undici");
const model = { api: "openai-completions" as const, provider: "llamacpp", id: "local-test", name: "Local test",
  baseUrl: "http://in-memory.invalid/v1", reasoning: false, input: ["text"] as ["text"],
  contextWindow: 262144, maxTokens: 100, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 } };
const context = normalizeContext({ messages: [{ role: "user", content: "hello", timestamp: 1 }] });
const frame = (delta: Record<string, unknown>, extra: Record<string, unknown> = {}) => ({
  id: "request-specific", object: "chat.completion.chunk", model: model.id,
  choices: [{ index: 0, delta, finish_reason: null }], ...extra,
});
const progress = (processed: number) => frame({ role: "assistant", content: null }, {
  prompt_progress: { total: 1000, cache: 0, processed, time_ms: processed },
});
const answer = () => [frame({ content: "OK" }), frame({}, {
  choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
  usage: { prompt_tokens: 1000, completion_tokens: 1, total_tokens: 1001 },
})];
const sse = (frames: unknown[]) => frames.map((value) => `data: ${JSON.stringify(value)}\n\n`).join("") + "data: [DONE]\n\n";

afterEach(() => { vi.unstubAllEnvs(); traces.length = 0; });

/** Real HTTP/1 bytes over a Duplex: exercises Undici timers/parser without sockets or the live engine. */
class MemoryConnection extends Duplex {
  private input = "";
  private started = false;
  private tick?: ReturnType<typeof setInterval>;
  private finish?: ReturnType<typeof setTimeout>;
  constructor(private readonly requests: Array<Record<string, unknown>>, private readonly commentPings = false) { super(); }
  _read(): void {}
  setNoDelay(): this { return this; }
  setKeepAlive(): this { return this; }
  ref(): this { return this; }
  unref(): this { return this; }
  _write(chunk: Buffer, _encoding: BufferEncoding, callback: (error?: Error | null) => void): void {
    this.input += chunk.toString();
    callback();
    if (this.started) return;
    const end = this.input.indexOf("\r\n\r\n");
    if (end < 0) return;
    const length = Number(/content-length: (\d+)/i.exec(this.input.slice(0, end))?.[1] ?? 0);
    if (this.input.length < end + 4 + length) return;
    this.started = true;
    const body = JSON.parse(this.input.slice(end + 4, end + 4 + length)) as Record<string, unknown>;
    this.requests.push(body);
    queueMicrotask(() => {
      this.push("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n");
      let processed = 0;
      if (body.return_progress || this.commentPings) this.tick = setInterval(() => this.send(
        body.return_progress ? `data: ${JSON.stringify(progress(++processed))}\n\n` : ":\n\n",
      ), 100);
      this.finish = setTimeout(() => {
        clearInterval(this.tick);
        this.send(sse(answer()));
        this.push("0\r\n\r\n");
      }, 2200);
    });
  }
  private send(text: string): void { this.push(`${Buffer.byteLength(text).toString(16)}\r\n${text}\r\n`); }
  _destroy(error: Error | null, callback: (error?: Error | null) => void): void {
    clearInterval(this.tick);
    clearTimeout(this.finish);
    callback(error);
  }
}

/** Inject the pinned client with a shortened inactivity deadline and no real networking. */
function transport(commentPings = false) {
  const requests: Array<Record<string, unknown>> = [];
  const client = new Client("http://in-memory.invalid", {
    connect: (_options, callback) => { queueMicrotask(() => callback(null, new MemoryConnection(requests, commentPings) as unknown as Socket)); },
    bodyTimeout: 500, headersTimeout: 500,
  });
  const fetch: typeof globalThis.fetch = (input, init) => httpFetch(input as string, {
    ...init, dispatcher: client,
  } as Parameters<typeof httpFetch>[1]) as unknown as Promise<Response>;
  return { requests, fetch, close: () => client.destroy() };
}

describe("local prefill progress", () => {
  it("keeps the original request alive past transport idleness without generated progress text", async () => {
    const wire = transport();
    const rawEvents: unknown[] = [];
    try {
      const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch: wire.fetch, maxRetries: 0,
        onProviderStreamEvent: (event) => { rawEvents.push(event); } }).result();
      expect(result.stopReason).toBe("stop");
      expect(result.content).toEqual([{ type: "text", text: "OK" }]);
      expect(result.usage.output).toBe(1);
      expect(wire.requests).toHaveLength(1);
      expect(wire.requests[0].return_progress).toBe(true);
      expect(rawEvents.some((event) => !!(event as { prompt_progress?: unknown }).prompt_progress)).toBe(true);
    } finally { await wire.close(); }
  }, 10_000);

  it("negative control: the same delayed answer terminates when native progress is disabled", async () => {
    vi.stubEnv("OFFICINA_NO_PREFILL_PROGRESS", "1");
    const wire = transport();
    try {
      const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch: wire.fetch, maxRetries: 0 }).result();
      expect(result.stopReason).toBe("error");
      expect(result.errorMessage).toContain("terminated");
      expect(traces.some((event) => event.detail?.includes("UND_ERR_BODY_TIMEOUT"))).toBe(true);
      expect(wire.requests).toHaveLength(1);
      expect(wire.requests[0].return_progress).toBeUndefined();
    } finally { await wire.close(); }
  }, 10_000);

  it("still honors user cancellation while healthy prefill is progressing", async () => {
    const wire = transport();
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 250);
    try {
      const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch: wire.fetch,
        signal: controller.signal, maxRetries: 0 }).result();
      expect(result.stopReason).toBe("aborted");
      expect(wire.requests).toHaveLength(1);
    } finally { clearTimeout(timer); await wire.close(); }
  });

  it("counterevidence control: working SSE comment keepalives also prevent inactivity timeout", async () => {
    vi.stubEnv("OFFICINA_NO_PREFILL_PROGRESS", "1");
    const wire = transport(true);
    try {
      const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch: wire.fetch, maxRetries: 0 }).result();
      expect(result.stopReason).toBe("stop");
      expect(wire.requests[0].return_progress).toBeUndefined();
      expect(wire.requests).toHaveLength(1);
    } finally { await wire.close(); }
  }, 10_000);

  it("preserves payload replacements, response callbacks and custom transport", async () => {
    const replacement = { model: model.id, messages: [], stream: true, return_progress: false, temperature: 0.17 };
    const hook = vi.fn((_payload: unknown, _model: unknown) => replacement);
    const response = vi.fn();
    const fetch = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response(sse([progress(100), ...answer()]), {
      headers: { "content-type": "text/event-stream" },
    }));
    const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch, onPayload: hook, onResponse: response }).result();
    expect(hook).toHaveBeenCalledTimes(1);
    expect(hook.mock.calls[0][0]).toMatchObject({ return_progress: true });
    expect(JSON.parse(fetch.mock.calls[0][1]!.body as string)).toEqual(replacement);
    expect(response).toHaveBeenCalledTimes(1);
    expect(result.stopReason).toBe("stop");
  });

  it("still reports real HTTP failures instead of waiting on unrelated engine activity", async () => {
    const fetch = vi.fn(async () => new Response(JSON.stringify({ error: { message: "engine unavailable" } }), { status: 503 }));
    const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch, maxRetries: 0 }).result();
    expect(result.stopReason).toBe("error");
    expect(result.errorMessage).toContain("engine unavailable");
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("does not alter cloud OpenAI requests", async () => {
    const fetch = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response(sse(answer())));
    const result = await streamLocalWithProgress({ ...model, provider: "caduceus" }, context, { apiKey: "test", fetch }).result();
    expect(result.stopReason).toBe("stop");
    expect(JSON.parse(fetch.mock.calls[0][1]!.body as string).return_progress).toBeUndefined();
  });

  it("leaves non-OpenAI local APIs unchanged", async () => {
    const fetch = vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) => new Response("unavailable", { status: 503 }));
    await streamLocalWithProgress({ ...model, api: "anthropic-messages" }, context, { apiKey: "test", fetch, maxRetries: 0 }).result();
    expect(JSON.parse(fetch.mock.calls[0][1]!.body as string).return_progress).toBeUndefined();
  });

  it("does not hide a genuinely broken stream after progress", async () => {
    const fetch = vi.fn(async () => new Response(new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(sse([progress(1)]).replace("data: [DONE]\n\n", "")));
        queueMicrotask(() => controller.error(new Error("actual stream failure")));
      },
    })));
    const result = await streamLocalWithProgress(model, context, { apiKey: "test", fetch, maxRetries: 0 }).result();
    expect(result.stopReason).toBe("error");
    expect(result.errorMessage).toContain("actual stream failure");
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it("ignores cloud endpoints and local telemetry-probe errors", () => {
    const errors = channel("undici:request:error");
    errors.publish({ request: { origin: "https://cloud.invalid", path: "/chat/completions" }, error: { code: "OTHER" } });
    errors.publish({ request: { origin: "http://in-memory.invalid", path: "/props" }, error: { code: "OTHER" } });
    expect(traces).toEqual([]);
  });
});
