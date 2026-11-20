// Admission probe tests (2026-11-20, lapis slot contention).
//
// probeAdmission() is the pre-send freshness check that closes the 700 ms
// /slots snapshot window in which a second session claims the only slot.
// I2 is the load-bearing invariant: a broken/slow engine must never block a
// local request — every failure mode degrades to {ok:false} and the caller
// falls back to snapshot logic.
import { createServer, type Server } from "node:http";
import { afterEach, describe, expect, it } from "vitest";
import {
  clearLocalContended,
  localContended,
  markLocalContended,
  probeAdmission,
} from "./engine.ts";

let server: Server | undefined;

afterEach(() => {
  server?.close();
  server = undefined;
  delete process.env.VITRIOL_BASE_URL;
  clearLocalContended();
});

/** Serve one canned /slots body (or hang) on an ephemeral port. */
async function serve(
  handler: (res: import("node:http").ServerResponse) => void,
): Promise<void> {
  server = createServer((req, res) => {
    if (req.url === "/slots") {
      handler(res);
      return;
    }
    res.writeHead(404);
    res.end();
  });
  await new Promise<void>((resolve) =>
    server!.listen(0, "127.0.0.1", () => {
      const addr = server!.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      process.env.VITRIOL_BASE_URL = `http://127.0.0.1:${port}`;
      resolve();
    }),
  );
}

describe("probeAdmission", () => {
  it("reports a busy slot from is_processing", async () => {
    await serve((res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify([{ id: 0, is_processing: true }]));
    });
    const probe = await probeAdmission(250);
    expect(probe).toEqual({ anyBusy: true, slotCount: 1, ok: true });
  });

  it("reports a free slot", async () => {
    await serve((res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify([{ id: 0, is_processing: false }]));
    });
    expect(await probeAdmission(250)).toEqual({
      anyBusy: false,
      slotCount: 1,
      ok: true,
    });
  });

  // I2 — fail-open on every failure mode.
  it("fails open on a 500", async () => {
    await serve((res) => {
      res.writeHead(500);
      res.end("boom");
    });
    expect(await probeAdmission(250)).toEqual({
      anyBusy: false,
      slotCount: 0,
      ok: false,
    });
  });

  it("fails open on bad JSON", async () => {
    await serve((res) => {
      res.writeHead(200, { "content-type": "application/json" });
      res.end("{not json");
    });
    expect((await probeAdmission(250)).ok).toBe(false);
  });

  it("fails open on a timeout within its own budget", async () => {
    await serve(() => {
      /* never respond */
    });
    const started = Date.now();
    const probe = await probeAdmission(120);
    expect(probe.ok).toBe(false);
    // The probe honours its budget: it must not outlive it materially.
    expect(Date.now() - started).toBeLessThan(1200);
  });

  it("fails open when the engine is down", async () => {
    process.env.VITRIOL_BASE_URL = "http://127.0.0.1:1"; // nothing listens
    expect((await probeAdmission(200)).ok).toBe(false);
  });
});

describe("local contention mark", () => {
  it("expires after its TTL", () => {
    const now = 1_000_000;
    markLocalContended(now, 3_000);
    expect(localContended(now)).toBe(true);
    expect(localContended(now + 2_999)).toBe(true);
    expect(localContended(now + 3_000)).toBe(false);
  });

  it("is clear on a fresh process (I5: never fabricated)", () => {
    clearLocalContended();
    expect(localContended()).toBe(false);
  });
});
