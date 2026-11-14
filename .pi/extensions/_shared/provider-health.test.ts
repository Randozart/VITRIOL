import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { rmSync } from "node:fs";
import { cooling, cooldownRemaining, healthSnapshot, recordFailure, recordRetryAfter, recordSuccess } from "./provider-health.ts";

const file = "/tmp/officina-health-test.json";
beforeEach(() => {
  process.env.OFFICINA_PROVIDER_HEALTH_FILE = file;
  rmSync(file, { force: true });
});
afterEach(() => {
  delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
  delete process.env.OFFICINA_LANE_COOLDOWN_MAX_MS;
  rmSync(file, { force: true });
});

describe("provider health", () => {
  it("persists cooldown and clears it on success", () => {
    recordFailure("caduceus", "dahl/MiniMax", 429, { retryAfterMs: 120_000 });
    expect(cooling("caduceus", "dahl/MiniMax")).toBe(true);
    expect(cooldownRemaining("caduceus", "dahl/MiniMax")).toBeGreaterThan(0);
    expect(healthSnapshot().models["caduceus/dahl/MiniMax"]?.failureCount).toBe(1);
    recordSuccess("caduceus", "dahl/MiniMax");
    expect(cooling("caduceus", "dahl/MiniMax")).toBe(false);
  });

  it("caps a 429 cooldown at 30 minutes by default", () => {
    const ms = recordRetryAfter("caduceus", "freeinference/x", { "retry-after": "27574" });
    expect(ms).toBe(1_800_000);
    expect(cooldownRemaining("caduceus", "freeinference/x")).toBeLessThanOrEqual(1_800_000);
    // Short hints are honoured as-is.
    expect(recordRetryAfter("caduceus", "dahl/y", { "retry-after": "120" })).toBe(120_000);
  });

  it("honours the cooldown ceiling override", () => {
    process.env.OFFICINA_LANE_COOLDOWN_MAX_MS = "60000";
    expect(recordRetryAfter("caduceus", "agentrouter/z", { "retry-after": "3600" })).toBe(60_000);
    expect(
      recordRetryAfter("caduceus", "agentrouter/w", {
        "retry-after": new Date(Date.now() + 4 * 3600_000).toUTCString(),
      }),
    ).toBeLessThanOrEqual(60_000);
  });

  it("cools the whole owner for auth failures", () => {
    recordFailure("caduceus", "glm/glm-4.7-flash", 401);
    expect(cooling("caduceus", "glm/glm-5.3-flash")).toBe(true);
  });
});
