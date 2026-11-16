import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { appendFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  lastRouteForSession,
  readRouteEntries,
  recordRoute,
  summarizeRoutes,
  todaySummary,
} from "./route-ledger.ts";

let dir: string;
const ledgerPathForTest = () => {
  const day = new Date().toISOString().slice(0, 10).replace(/-/g, "");
  return join(dir, `route-ledger-${day}.jsonl`);
};
beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "ledger-test-"));
  process.env.OFFICINA_ROUTE_LEDGER_DIR = dir;
});
afterEach(() => {
  delete process.env.OFFICINA_ROUTE_LEDGER_DIR;
  rmSync(dir, { recursive: true, force: true });
});

describe("route ledger", () => {
  it("appends privacy-safe route entries and summarizes them", () => {
    recordRoute({ ts: new Date().toISOString(), reason: "user", tier: "cloud", provider: "caduceus", model: "deepseek", tokensIn: 10, tokensOut: 5, outcome: "ok" });
    recordRoute({ ts: new Date().toISOString(), reason: "retry", tier: "local", provider: "llamacpp", model: "Lapis", tokensIn: 7, tokensOut: 3, outcome: "error" });
    const rows = readRouteEntries(2);
    expect(rows).toHaveLength(2);
    const s = summarizeRoutes(rows);
    expect(s.requests).toBe(2);
    expect(s.localRequests).toBe(1);
    expect(s.cloudRequests).toBe(1);
    expect(s.failures).toBe(1);
    expect(s.tokensIn).toBe(17);
    expect(s.tokensOut).toBe(8);
    expect(JSON.stringify(rows)).not.toContain("api_key");
    expect(todaySummary().requests).toBe(2);
  });
});

describe("lastRouteForSession", () => {
  const put = (sessionId: string, model: string, out = 1) =>
    recordRoute({
      ts: new Date().toISOString(),
      sessionId,
      reason: "user",
      tier: "cloud",
      provider: "caduceus",
      model,
      tokensIn: 10,
      tokensOut: out,
      outcome: "ok",
    });

  it("returns only this session's newest entry (P3)", () => {
    put("s-a", "m1");
    put("s-b", "m2");
    put("s-a", "m3", 4);
    expect(lastRouteForSession("s-a", { cacheMs: 0 })?.model).toBe("m3");
    expect(lastRouteForSession("s-b", { cacheMs: 0 })?.model).toBe("m2");
    expect(lastRouteForSession("s-a", { cacheMs: 0 })?.tokensOut).toBe(4);
    expect(lastRouteForSession("nobody", { cacheMs: 0 })).toBeNull();
    expect(lastRouteForSession("", { cacheMs: 0 })).toBeNull();
  });

  it("survives a torn tail line and still finds the entry", () => {
    put("s-a", "kept");
    appendFileSync(ledgerPathForTest(), '{"sessionId":"s-a","model":"tor');
    expect(lastRouteForSession("s-a", { cacheMs: 0 })?.model).toBe("kept");
  });

  it("caches on the file mtime so a 700 ms tick does not re-read", () => {
    put("s-a", "first");
    let reads = 0;
    const spy = () => {
      reads += 1;
      return Date.now();
    };
    expect(lastRouteForSession("s-a", { cacheMs: 60_000, now: spy })?.model).toBe("first");
    expect(lastRouteForSession("s-a", { cacheMs: 60_000, now: spy })?.model).toBe("first");
    expect(reads).toBe(2); // timestamps only; the file is read once
  });
});
