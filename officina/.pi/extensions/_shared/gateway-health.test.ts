import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  markGatewayCooldown,
  markGatewayInvalid,
  markModelLimit,
  modelLimit,
  parseContextLimit,
  ownerOfLane,
  parseCooldownFromError,
  parseModelNotFound,
  readGatewayMarks,
} from "./gateway-health.ts";

const now = Date.now();
let dir = "";

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "gw-health-"));
  process.env.OFFICINA_GATEWAY_HEALTH_FILE = join(dir, "gateway-health.json");
  delete process.env.OFFICINA_LANE_COOLDOWN_MAX_MS;
});

afterEach(() => {
  delete process.env.OFFICINA_GATEWAY_HEALTH_FILE;
  delete process.env.OFFICINA_LANE_COOLDOWN_MAX_MS;
  rmSync(dir, { recursive: true, force: true });
});

describe("parseCooldownFromError", () => {
  it("reads reset_seconds from the gateway 429 body", () => {
    expect(parseCooldownFromError('{"code":"model_cooldown","reset_seconds":21883}', now)).toBe(
      1_800_000, // capped at 30 min
    );
    expect(parseCooldownFromError('{"reset_seconds":120}', now)).toBe(120_000);
  });

  it("reads reset_at / retry_after ISO stamps", () => {
    const iso = new Date(now + 5 * 60_000).toISOString();
    expect(parseCooldownFromError(`{"retry_after":"${iso}"}`, now)).toBeGreaterThan(250_000);
    expect(parseCooldownFromError(`{"reset_at":"${iso}"}`, now)).toBeGreaterThan(250_000);
  });

  it("ignores unrelated text and honours the ceiling override", () => {
    expect(parseCooldownFromError("plain error", now)).toBeUndefined();
    expect(parseCooldownFromError(undefined, now)).toBeUndefined();
    process.env.OFFICINA_LANE_COOLDOWN_MAX_MS = "60000";
    expect(parseCooldownFromError('{"reset_seconds":3600}', now)).toBe(60_000);
  });
});

describe("gateway marks", () => {
  it("merges model and owner marks and reads them back", () => {
    markGatewayCooldown({
      model: "freeinference/deepseek-v4-flash",
      owner: "freeinference",
      untilMs: now + 10 * 60_000,
      reason: "model_cooldown",
    });
    const marks = readGatewayMarks(now);
    expect(marks.models.has("freeinference/deepseek-v4-flash")).toBe(true);
    expect(marks.owners.has("freeinference")).toBe(true);
    expect(marks.models.has("dahl/MiniMax-M2.7")).toBe(false);
  });

  it("drops expired marks", () => {
    markGatewayCooldown({ model: "dahl/x", untilMs: now + 60_000, reason: "t" });
    expect(readGatewayMarks(now).models.has("dahl/x")).toBe(true);
    expect(readGatewayMarks(now + 120_000).models.has("dahl/x")).toBe(false);
  });

  it("clamps marks to the cooldown ceiling", () => {
    const before = Date.now();
    const until = markGatewayCooldown({
      model: "agentrouter/x",
      untilMs: before + 24 * 3600_000,
      reason: "day long",
    });
    expect(until - before).toBeLessThanOrEqual(1_801_000);
    expect(until - before).toBeGreaterThan(1_790_000);
  });

  it("derives the owner from a lane id", () => {
    expect(ownerOfLane("freeinference/deepseek-v4-flash")).toBe("freeinference");
    expect(ownerOfLane("CADUCEUS/Dahl/x")).toBe("caduceus");
    expect(ownerOfLane("")).toBe("");
  });
});

describe("model_not_found self-heal", () => {
  const body =
    '404: {"message":"[404]: This model is not currently offered: moonshotai/Kimi-K2.6. ' +
    'Available models: MiniMaxAI/MiniMax-M2.7, deepseek-ai/DeepSeek-V4-Flash-0731, ' +
    'zai-org/GLM-5.3-Flash.","code":"model_not_found"}';

  it("parses the missing id and the upstream alternatives", () => {
    expect(parseModelNotFound(body)).toEqual({
      missing: "moonshotai/Kimi-K2.6",
      alternatives: [
        "MiniMaxAI/MiniMax-M2.7",
        "deepseek-ai/DeepSeek-V4-Flash-0731",
        "zai-org/GLM-5.3-Flash",
      ],
    });
    expect(parseModelNotFound("429 rate limited")).toBeUndefined();
  });

  it("marks the stale lane invalid and reads it back", () => {
    markGatewayInvalid({ model: "dahl/moonshotai/Kimi-K2.6", reason: "not offered" });
    const marks = readGatewayMarks();
    expect(marks.invalid.has("dahl/moonshotai/Kimi-K2.6")).toBe(true);
    expect(marks.models.size).toBe(0);
  });

  it("invalid marks live longer than a 429 mark but are still bounded", () => {
    const until = markGatewayInvalid({ model: "dahl/x", reason: "stale" });
    const delta = until - Date.now();
    expect(delta).toBeGreaterThan(1_800_000);
    expect(delta).toBeLessThanOrEqual(6 * 3600_000);
    expect(readGatewayMarks(Date.now() + 7 * 3600_000).invalid.has("dahl/x")).toBe(false);
  });
});

describe("learned context limits", () => {
  it("parses the provider 400 message", () => {
    const text =
      "[400]: This model's maximum context length is 180000 tokens. However, you requested 8192 output tokens";
    expect(parseContextLimit(text)).toBe(180000);
    expect(parseContextLimit("429 rate limited")).toBeUndefined();
    expect(parseContextLimit('{"maximum context length is 512 tokens"}')).toBeUndefined();
  });

  it("remembers the smaller of repeated limits", () => {
    markModelLimit("dahl/MiniMaxAI/MiniMax-M2.7", 180000);
    markModelLimit("dahl/MiniMaxAI/MiniMax-M2.7", 190000);
    expect(modelLimit("dahl/MiniMaxAI/MiniMax-M2.7")).toBe(180000);
    markModelLimit("dahl/MiniMaxAI/MiniMax-M2.7", 160000);
    expect(modelLimit("dahl/MiniMaxAI/MiniMax-M2.7")).toBe(160000);
  });
});
