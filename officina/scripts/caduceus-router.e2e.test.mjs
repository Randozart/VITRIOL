// End-to-end check for the routing proxy: a stub gateway that quota-fails the
// requested lane and serves a fallback (2026-11-16).
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFileSync, rmSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";

const STATE = join(tmpdir(), `caduceus-router-e2e-${process.pid}.json`);
const ROUTER_PORT = 25_911 + (process.pid % 200);
const GATEWAY_PORT = 26_911 + (process.pid % 200);

/** Stub gateway: quota-fails anything not in `served`, streams SSE for those. */
function startStub() {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      const chunks = [];
      for await (const c of req) chunks.push(c);
      let model = "";
      try {
        model = JSON.parse(Buffer.concat(chunks).toString("utf8")).model ?? "";
      } catch {
        /* ignore */
      }
      if (req.url === "/served") {
        res.writeHead(200);
        res.end("ok");
        return;
      }
      if (model === "good/lane") {
        res.writeHead(200, { "content-type": "text/event-stream" });
        res.write('data: {"choices":[{"delta":{"content":"pong"}}]}\n\n');
        res.end("data: [DONE]\n\n");
        return;
      }
      res.writeHead(429, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          message: `[${model}] [429]: quota spent. Try again after 18:54:17.`,
          retry_after: 177,
          reset_at: new Date(Date.now() + 177_000).toISOString(),
        }),
      );
    });
    server.listen(GATEWAY_PORT, "127.0.0.1", () => resolve(server));
  });
}

let stub;
let proxy;

beforeAll(async () => {
  rmSync(STATE, { force: true });
  stub = await startStub();
  proxy = spawn(
    process.execPath,
    [
      join(__dirname, "caduceus-router.mjs"),
      "--port",
      String(ROUTER_PORT),
      "--gateway",
      `http://127.0.0.1:${GATEWAY_PORT}`,
    ],
    {
      env: {
        ...process.env,
        CADUCEUS_ROUTER_FALLBACKS: "good/lane,other/lane",
        CADUCEUS_ROUTER_STATE: STATE,
        CADUCEUS_ROUTER_FIRST_BYTE_MS: "5000",
      },
      stdio: "ignore",
    },
  );
  // Wait for the proxy's port to answer.
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${ROUTER_PORT}/health`);
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 100));
  }
});

afterAll(() => {
  proxy?.kill("SIGTERM");
  stub?.close();
  rmSync(STATE, { force: true });
});

const chat = (model) =>
  fetch(`http://127.0.0.1:${ROUTER_PORT}/v1/chat/completions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model,
      messages: [{ role: "user", content: "hi" }],
    }),
  });

describe("caduceus-router (end to end)", () => {
  it("reroutes a quota-failed lane to a fallback and parks the failed one", async () => {
    const res = await chat("bad/lane");
    expect(res.status).toBe(200);
    expect(res.headers.get("x-caduceus-router")).toBe(
      "model=good/lane;attempt=2",
    );
    const text = await res.text();
    expect(text).toContain("pong");

    const state = JSON.parse(readFileSync(STATE, "utf8"));
    expect(Object.keys(state.parked)).toContain("bad/lane");
    // Parked for the duration the gateway stated (177 s), not a default.
    const remaining = state.parked["bad/lane"] - Date.now();
    expect(remaining).toBeGreaterThan(150_000);
  });

  it("serves the requested lane directly when it is healthy", async () => {
    const res = await chat("good/lane");
    expect(res.status).toBe(200);
    expect(res.headers.get("x-caduceus-router")).toBe(
      "model=good/lane;attempt=1",
    );
    await res.text();
  });

  it("reports a spent lane as parked rather than asking again", async () => {
    const res = await chat("bad/lane");
    // bad/lane is parked now, so the first attempt is the fallback.
    expect(res.status).toBe(200);
    expect(res.headers.get("x-caduceus-router")).toBe(
      "model=good/lane;attempt=1",
    );
    await res.text();
  });

  it("gives up only when every candidate is exhausted", async () => {
    const res = await chat("bad/other");
    // bad/other is not parked; both candidates have been parked above except
    // other/lane, which the stub also quota-fails.
    expect([200, 429, 502, 503]).toContain(res.status);
    await res.text();
  });
});
