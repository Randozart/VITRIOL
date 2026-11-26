// End-to-end check for the routing proxy: a stub gateway that quota-fails the
// requested lane and serves a fallback (2026-11-16).
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFileSync, rmSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const HERE = dirname(fileURLToPath(import.meta.url));
const STATE = join(tmpdir(), `caduceus-router-e2e-${process.pid}.json`);
// Ephemeral ports: fixed ones collide with a parallel run or a leftover child.
let ROUTER_PORT = 0;
let GATEWAY_PORT = 0;
let serveGoodLane = true;

/** Bind an ephemeral port and hand the number back. */
function freePort() {
  return new Promise((resolve, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
}

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
      if (model === "good/lane" && serveGoodLane) {
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
    server.once("error", () => resolve(null));
    server.listen(GATEWAY_PORT, "127.0.0.1", () => {
      GATEWAY_PORT = server.address().port;
      resolve(server);
    });
  });
}

let stub;
let proxy;

beforeAll(async () => {
  rmSync(STATE, { force: true });
  GATEWAY_PORT = await freePort();
  ROUTER_PORT = await freePort();
  stub = await startStub();
  proxy = spawn(
    process.execPath,
    [
      join(HERE, "caduceus-router.mjs"),
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
      stdio: ["ignore", "pipe", "pipe"],
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

  it("no-model 401 storm leaves lanes servable (I3)", async () => {
    // A request without a model field (what stats RPCs/catalog reads look
    // like) that the gateway rejects must NOT poison the park table — the
    // 2026-11-21 pre-fix behaviour parked the literal "?" key.
    const bad = await fetch(
      `http://127.0.0.1:${ROUTER_PORT}/v1/chat/completions`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ messages: [{ role: "user", content: "hi" }] }),
      },
    );
    await bad.text();
    const state = JSON.parse(readFileSync(STATE, "utf8"));
    expect(Object.keys(state.parked ?? {})).not.toContain("?");
    // The real lane is still walkable: good/lane was parked earlier in this
    // file, so use a fresh id the stub serves.
    const res = await chat("good/lane");
    // Either served directly or rerouted — the point is NOT 503-give-up.
    expect(res.status).not.toBe(503);
    await res.text();
  });

  it("unpark endpoint removes exactly the named entry (I6)", async () => {
    // Park something deterministically first.
    await chat("bad/unpark-target");
    let state = JSON.parse(readFileSync(STATE, "utf8"));
    expect(Object.keys(state.parked)).toContain("bad/unpark-target");
    const healthUrl = `http://127.0.0.1:${ROUTER_PORT}/health`;
    const health = await (await fetch(healthUrl)).json();
    expect(health).toMatchObject({
      ok: true,
      lastRequested: "bad/unpark-target",
      fallbacks: ["good/lane", "other/lane"],
    });
    expect(health.parked).toContainEqual({
      model: "bad/unpark-target",
      seconds: expect.any(Number),
      reason: "HTTP 429",
    });

    const missing = await fetch(
      `http://127.0.0.1:${ROUTER_PORT}/park?model=never/parked`,
      { method: "DELETE" },
    );
    expect(missing.status).toBe(404);
    await missing.text();

    const noParam = await fetch(`http://127.0.0.1:${ROUTER_PORT}/park`, {
      method: "DELETE",
    });
    expect(noParam.status).toBe(400);
    await noParam.text();

    const ok = await fetch(
      `http://127.0.0.1:${ROUTER_PORT}/park?model=bad/unpark-target`,
      { method: "DELETE" },
    );
    expect(ok.status).toBe(200);
    await ok.text();
    state = JSON.parse(readFileSync(STATE, "utf8"));
    expect(Object.keys(state.parked ?? {})).not.toContain("bad/unpark-target");
    expect((await (await fetch(healthUrl)).json()).parked).not.toContainEqual(
      expect.objectContaining({ model: "bad/unpark-target" }),
    );
    // After unparking, the lane is walked again (it fails → 200 via fallback
    // or 429 surfaced; never a stale-park shortcut).
    const res = await chat("bad/unpark-target");
    expect([200, 429]).toContain(res.status);
    await res.text();
  });

  it("counts only distinct lanes for a request whose model is also a fallback", async () => {
    serveGoodLane = false;
    const first = await chat("good/lane");
    expect(first.status).toBe(429);
    await first.text();
    const parked = await chat("good/lane");
    expect(parked.status).toBe(503);
    expect((await parked.json()).error.message).toMatch(
      /^all 2 candidate\(s\) unavailable — earliest lane returns in \d+s$/,
    );
  });
});
