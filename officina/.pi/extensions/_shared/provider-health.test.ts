import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { rmSync } from "node:fs";
import { cooling, cooldownRemaining, healthSnapshot, recordFailure, recordSuccess } from "./provider-health.ts";

const file = "/tmp/officina-health-test.json";
beforeEach(() => {
  process.env.OFFICINA_PROVIDER_HEALTH_FILE = file;
  rmSync(file, { force: true });
});
afterEach(() => {
  delete process.env.OFFICINA_PROVIDER_HEALTH_FILE;
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

  it("cools the whole owner for auth failures", () => {
    recordFailure("caduceus", "glm/glm-4.7-flash", 401);
    expect(cooling("caduceus", "glm/glm-5.3-flash")).toBe(true);
  });
});
