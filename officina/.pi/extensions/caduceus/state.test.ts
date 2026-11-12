// Tests for the caduceus state-file builder (Rust-modal contract: snake_case).
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { buildCaduceusState, writeCaduceusState, type CaduceusStateInput } from "./state.ts";

const input = (over: Partial<CaduceusStateInput> = {}): CaduceusStateInput => ({
  base_url: "http://127.0.0.1:20128/v1",
  gateway_up: true,
  catalog_count: 123,
  catalog_fetched_at: "2026-11-11T10:00:00.000Z",
  connections: { active: ["a", "b"], backoff: ["c"], hard_down: ["d"] },
  cooling: 2,
  inference_key: "...a1b2",
  keys: [{ provider: "zai", env: "ZAI_KEY", value: "...c3d4", source: "env-file" }],
  budget: {
    daily_tokens: 1_200_000,
    daily_cap: 2_000_000,
    monthly_tokens: 8_000_000,
    monthly_cap: 40_000_000,
    allowed: true,
    paid_allowed: false,
  },
  ...over,
});

describe("buildCaduceusState", () => {
  it("stamps ts and keeps every snake_case key (serde contract)", () => {
    const s = buildCaduceusState({ ...input(), now: 1731_000_000_000 });
    expect(s.ts).toBe(1731_000_000_000);
    // exact key set the Rust struct deserializes
    expect(Object.keys(s).sort()).toEqual([
      "base_url", "budget", "catalog_count", "catalog_fetched_at", "connections",
      "cooling", "gateway_up", "inference_key", "keys", "ts",
    ]);
    expect(Object.keys(s.budget).sort()).toEqual([
      "allowed", "daily_cap", "daily_tokens", "monthly_cap", "monthly_tokens", "paid_allowed",
    ]);
    expect(Object.keys(s.connections).sort()).toEqual(["active", "backoff", "hard_down"]);
    expect((s as unknown as { now?: number }).now).toBeUndefined();
  });

  it("defaults ts to now", () => {
    const before = Date.now();
    const s = buildCaduceusState(input());
    expect(s.ts).toBeGreaterThanOrEqual(before);
  });
});

describe("writeCaduceusState", () => {
  const dir = mkdtempSync(join(tmpdir(), "caduceus-state-"));
  afterAll(() => rmSync(dir, { recursive: true, force: true }));

  it("atomic write round-trips as JSON", () => {
    const path = join(dir, "caduceus-state.json");
    const state = buildCaduceusState({ ...input(), now: 42 });
    writeCaduceusState(path, state);
    const back = JSON.parse(readFileSync(path, "utf8"));
    expect(back).toEqual(state);
    expect(back.ts).toBe(42);
    expect(back.connections.hard_down).toEqual(["d"]);
    expect(back.budget.allowed).toBe(true);
  });
});
