// End-to-end: the decision model reorders the proxy's lane choice (2026-11-16).
//
// Stub gateway serves two lanes; a stub Decisions endpoint prefers the second.
// The proxy must (a) ask once, (b) try the preferred lane first, (c) still work
// when the decision endpoint is down.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

const HERE = dirname(fileURLToPath(import.meta.url));
let ROUTER_PORT = 0;
let GATEWAY_PORT = 0;
let DECISION_PORT = 0;
let dir = "";
let stub;
let decisions;
let proxy;
let decisionCalls = 0;
let decisionMode = "prefer-second"; // or "down"

const freePort = () =>
  new Promise((resolve, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });

function startGateway() {
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
      if (req.url === "/v1/models") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(
          JSON.stringify({
            data: [
              { id: "good/one", context_length: 128000 },
              { id: "good/two", context_length: 200000 },
            ],
          }),
        );
        return;
      }
      if (model === "good/one" || model === "good/two") {
        res.writeHead(200, { "content-type": "text/event-stream" });
        res.end(`data: {"served":"${model}"}\n\n`);
        return;
      }
      res.writeHead(429, { "content-type": "application/json" });
      res.end(JSON.stringify({ message: "quota", retry_after: 60 }));
    });
    server.listen(GATEWAY_PORT, "127.0.0.1", () => {
      GATEWAY_PORT = server.address().port;
      resolve(server);
    });
  });
}

function startDecisions() {
  return new Promise((resolve) => {
    const server = createServer(async (req, res) => {
      const chunks = [];
      for await (const c of req) chunks.push(c);
      decisionCalls += 1;
      if (decisionMode === "down") {
        res.writeHead(500);
        res.end("{}");
        return;
      }
      const body = JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
      const keys = Object.keys(body?.questions?.lane?.criteria ?? {});
      // Prefer the LAST candidate, so a reorder is observable.
      const probabilities = {};
      keys.forEach(
        (k, i) => (probabilities[k] = i === keys.length - 1 ? 0.9 : 0.05),
      );
      res.writeHead(200, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          answers: {
            lane: { choice: keys[0], probabilities, confidence: 0.9 },
          },
        }),
      );
    });
    server.listen(DECISION_PORT, "127.0.0.1", () => {
      DECISION_PORT = server.address().port;
      resolve(server);
    });
  });
}

const startProxy = () =>
  new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      [
        join(HERE, "caduceus-router.mjs"),
        "--port",
        String(ROUTER_PORT),
        "--gateway",
        `http://127.0.0.1:${GATEWAY_PORT}`,
        "--decision",
        "1",
        "--decision-url",
        `http://127.0.0.1:${DECISION_PORT}/v1beta/decisions`,
        "--decision-floor",
        "0.5",
      ],
      {
        env: {
          ...process.env,
          CADUCEUS_ROUTER_FALLBACKS: "good/one,good/two",
          CADUCEUS_ROUTER_STATE: join(dir, "state.json"),
          GEMINI_API_KEY: "AI" + "za" + "T".repeat(35),
        },
        stdio: "ignore",
      },
    );
    child.on("error", reject);
    const deadline = Date.now() + 10_000;
    const poll = async () => {
      try {
        const res = await fetch(`http://127.0.0.1:${ROUTER_PORT}/health`);
        if (res.ok) return resolve(child);
      } catch {
        /* not up yet */
      }
      if (Date.now() > deadline)
        return reject(new Error("proxy did not start"));
      setTimeout(poll, 100);
    };
    poll();
  });

beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), "caduceus-router-decision-"));
  GATEWAY_PORT = await freePort();
  ROUTER_PORT = await freePort();
  DECISION_PORT = await freePort();
  stub = await startGateway();
  decisions = await startDecisions();
  proxy = await startProxy();
}, 30_000);

afterAll(() => {
  proxy?.kill("SIGTERM");
  stub?.close();
  decisions?.close();
  rmSync(dir, { recursive: true, force: true });
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

describe("decision-model lane choice (end to end)", () => {
  it("tries the lane the model prefers, first", async () => {
    const res = await chat("bad/requested");
    expect(res.status).toBe(200);
    // R-A1: the REQUESTED model is always tried first (it is what the client —
    // and possibly the operator's pin — asked for). The model's pick goes to the
    // front of the fallback tail, so it is attempt 2, ahead of good/one.
    expect(res.headers.get("x-caduceus-router")).toBe(
      "model=good/two;attempt=2",
    );
    await res.text();
    expect(decisionCalls).toBe(1);
  });

  it("memoises within the window (a burst is one call)", async () => {
    // A request for a WORKING lane parks nothing, so the candidate set — and
    // therefore the memo key — is identical on the retry.
    const first = await chat("good/one");
    expect(first.status).toBe(200);
    await first.text();
    const after = decisionCalls;
    const second = await chat("good/one");
    expect(second.status).toBe(200);
    await second.text();
    expect(decisionCalls).toBe(after);
  });

  it("falls back to the deterministic order when the decision endpoint fails", async () => {
    decisionMode = "down";
    // A different model id defeats the memo.
    const res = await chat("bad/other");
    expect(res.status).toBe(200);
    // Deterministic order: the requested model (429) then the first fallback.
    expect(res.headers.get("x-caduceus-router")).toBe(
      "model=good/one;attempt=2",
    );
    await res.text();
    decisionMode = "prefer-second";
  });
});
