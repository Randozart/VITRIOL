import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readRouteEntries, recordRoute, summarizeRoutes, todaySummary } from "./route-ledger.ts";

let dir: string;
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
