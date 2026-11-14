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
  parseOverflowPromptTokens,
  parseOwnerPause,
  markOwnerPause,
  clearOwnerPause,
  markOffered,
  markAlias,
  markProbe,
  acquireProbeLease,
  releaseProbeLease,
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

describe("gateway context-overflow variants", () => {
  it("reads the limit from the gateway wrapper", () => {
    expect(
      parseContextLimit(
        "Input exceeds context window for freeinference/qwen3.6-35b: estimated 183106 input tokens, limit 128000",
      ),
    ).toBe(128000);
  });

  it("reads the prompt size quoted by the provider", () => {
    expect(parseOverflowPromptTokens("(parameter=input_tokens, value=171809)")).toBe(171809);
    expect(
      parseOverflowPromptTokens("estimated 183106 input tokens, limit 128000"),
    ).toBe(183106);
    expect(
      parseOverflowPromptTokens("your prompt contains at least 171809 input tokens"),
    ).toBe(171809);
  });
});

describe("owner pause (400 key circuit)", () => {
  const body =
    "[400]: This model's maximum context length is 180000 tokens, and this request may not fit. " +
    "Several requests from this key went over the limit, so requests that may not fit are paused " +
    "for 11 minutes. Requests under about 56 KB still go through";

  it("parses the pause and the degraded safe size", () => {
    const p = parseOwnerPause(body);
    expect(p?.pauseMs).toBe(11 * 60_000);
    expect(p?.safeBytes).toBe(56 * 1024);
    expect(parseOwnerPause("paused for 30 seconds")).toMatchObject({ pauseMs: 60_000 });
    expect(parseOwnerPause("429 rate limited")).toBeUndefined();
  });

  it("benches the owner and exposes safe bytes until cleared", () => {
    markOwnerPause("dahl", { pauseMs: 5 * 60_000, safeBytes: 56 * 1024 }, "test");
    let marks = readGatewayMarks();
    expect(marks.pausedOwners.has("dahl")).toBe(true);
    expect(marks.safeBytesByOwner["dahl"]).toBe(56 * 1024);
    clearOwnerPause("dahl");
    marks = readGatewayMarks();
    expect(marks.pausedOwners.has("dahl")).toBe(false);
  });

  it("remembers offered ids and remaps", () => {
    markOffered("dahl", ["MiniMaxAI/MiniMax-M2.7", "zai-org/GLM-5.3-Flash"]);
    markAlias("dahl/moonshotai/Kimi-K2.6", "dahl/MiniMaxAI/MiniMax-M2.7");
    const marks = readGatewayMarks();
    expect(marks.offered["dahl"]).toContain("MiniMaxAI/MiniMax-M2.7");
    expect(marks.aliases["dahl/moonshotai/Kimi-K2.6"]).toBe("dahl/MiniMaxAI/MiniMax-M2.7");
  });
});

describe("probe facts + leader lease", () => {
  it("stores probe results and reads them back", () => {
    markProbe("dahl/MiniMaxAI/MiniMax-M2.7", {
      kind: "ok",
      okAt: Date.now(),
      latencyMs: 120,
      failCount: 0,
      nextProbeAt: Date.now() + 60_000,
    });
    const marks = readGatewayMarks();
    expect(marks.probes["dahl/MiniMaxAI/MiniMax-M2.7"]?.kind).toBe("ok");
    expect(marks.probes["dahl/MiniMaxAI/MiniMax-M2.7"]?.latencyMs).toBe(120);
  });

  it("grants one prober and takes over expired leases", () => {
    process.env.OFFICINA_PROBE_TICK_MS = "0";
    expect(acquireProbeLease(1111, 20_000)).toBe(true);
    expect(acquireProbeLease(2222, 20_000)).toBe(false);
    // Simulate expiry by writing a past lease directly.
    const path = process.env.OFFICINA_GATEWAY_HEALTH_FILE!;
    const fs = require("node:fs") as typeof import("node:fs");
    const doc = JSON.parse(fs.readFileSync(path, "utf8"));
    doc.probeLease = { pid: 1111, until: Date.now() - 1 };
    fs.writeFileSync(path, JSON.stringify(doc));
    expect(acquireProbeLease(2222, 20_000)).toBe(true);
    releaseProbeLease(2222);
    expect(readGatewayMarks().probeLease?.pid).not.toBe(2222);
  });
});

describe("provider-body robustness (property-style)", () => {
  const bodies = [
    "[400]: This model's maximum context length is 180000 tokens. However, you requested 8192 output tokens and your prompt contains at least 171809 input tokens (parameter=input_tokens, value=171809)",
    "[400]: This model's maximum context length is 180000 tokens, and this request may not fit. Several requests from this key went over the limit, so requests that may not fit are paused for 11 minutes. Requests under about 56 KB still go through",
    '429: {"message":"All credentials for model deepseek-v4-flash are cooling down","code":"model_cooldown","reset_seconds":20054,"retry_after":"2026-11-14T23:59:59.660Z"}',
    '[404]: This model is not currently offered: moonshotai/Kimi-K2.6. Available models: MiniMaxAI/MiniMax-M2.7, deepseek-ai/DeepSeek-V4-Flash-0731, zai-org/GLM-5.3-Flash.',
    "Input exceeds context window for freeinference/qwen3.6-35b: estimated 183106 input tokens, limit 128000.",
  ];

  function rng(seed: number): () => number {
    let state = seed >>> 0;
    return () => {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      return state / 0x100000000;
    };
  }

  it("never throws and keeps values bounded under mutations", () => {
    const random = rng(0x5eed);
    const cap = 1_800_000;
    for (const body of bodies) {
      const variants: string[] = [body, body.slice(0, Math.floor(body.length / 3)), "", "\u0000\uffff"];
      for (let i = 0; i < 40; i += 1) {
        const at = Math.floor(random() * body.length);
        const digit = String(Math.floor(random() * 10));
        variants.push(`${body.slice(0, at)}${digit}${body.slice(at + 1)}`);
        variants.push(`${body.slice(0, at)}${body.slice(at + 2)}`);
      }
      for (const text of variants) {
        expect(() => parseCooldownFromError(text, Date.now())).not.toThrow();
        expect(() => parseContextLimit(text)).not.toThrow();
        expect(() => parseOverflowPromptTokens(text)).not.toThrow();
        expect(() => parseOwnerPause(text)).not.toThrow();
        expect(() => parseModelNotFound(text)).not.toThrow();
        const cooldown = parseCooldownFromError(text, Date.now());
        if (cooldown !== undefined) {
          expect(cooldown).toBeGreaterThanOrEqual(60_000);
          expect(cooldown).toBeLessThanOrEqual(cap);
        }
        const limit = parseContextLimit(text);
        if (limit !== undefined) {
          expect(limit).toBeGreaterThanOrEqual(1024);
          expect(limit).toBeLessThan(10_000_000);
        }
        const pause = parseOwnerPause(text);
        if (pause?.pauseMs !== undefined) {
          expect(pause.pauseMs).toBeGreaterThanOrEqual(60_000);
          expect(pause.pauseMs).toBeLessThanOrEqual(3_600_000);
        }
      }
    }
  });
});
