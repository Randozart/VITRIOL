// Tests for the caduceus state-file builder (Rust-modal contract: snake_case).
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { buildCaduceusState, buildProviderRows, writeCaduceusState, type CaduceusStateInput } from "./state.ts";

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
    expect(s.version).toBe(2);
    // exact key set the Rust struct deserializes
    expect(Object.keys(s).sort()).toEqual([
      "base_url", "budget", "candidates", "catalog_count", "catalog_fetched_at", "connections",
      "cooling", "gateway_up", "inference_key", "keys", "providers", "ts", "version",
    ]);
    expect(Object.keys(s.budget).sort()).toEqual([
      "allowed", "daily_cap", "daily_tokens", "monthly_cap", "monthly_tokens", "paid_allowed",
    ]);
    expect(Object.keys(s.connections).sort()).toEqual(["active", "backoff", "hard_down"]);
    expect((s as unknown as { now?: number }).now).toBeUndefined();
  });

  it("defaults providers/candidates to empty arrays (v1-state parity)", () => {
    const s = buildCaduceusState(input());
    expect(s.providers).toEqual([]);
    expect(s.candidates).toEqual([]);
  });

  it("defaults ts to now", () => {
    const before = Date.now();
    const s = buildCaduceusState(input());
    expect(s.ts).toBeGreaterThanOrEqual(before);
  });
});

describe("buildProviderRows", () => {
  const facts = () => ({
    catalog: [
      { id: "GLM", name: "GLM", website: "https://glm", category: "api-key", hasFree: true, freeNote: "flash tier", authHint: "key" },
      { id: "freeinference", name: "FreeInference", category: "api-key", hasFree: true },
      { id: "dahl", name: "Dahl", category: "api-key", hasFree: true },
    ],
    accounts: [
      { provider: "glm", name: "GLM", credential_env: "GLM_API_KEY", website: "https://glm", enabled: true },
      { provider: "opencode-go", name: "OpenCode Go", credential_env: "OPENCODE_API_KEY", enabled: false },
    ],
    oauth: [{ id: "codex", name: "OpenAI Codex (OAuth)", flow: "callback" }],
    envNames: new Set(["GLM_API_KEY"]),
    buckets: { active: ["glm"], backoff: ["freeinference"], hard_down: ["dahl"] },
    freeCounts: new Map([["glm", 3]]),
    signup: { glm: { status: "keyed" } },
  });

  it("merges accounts, catalog, oauth, env and gateway facts", () => {
    const rows = buildProviderRows(facts());
    const byId = Object.fromEntries(rows.map((r) => [r.id, r]));
    expect(byId.glm).toMatchObject({
      enabled: true,
      key_present: true,
      gateway: "active",
      free_models: 3,
      has_free: true,
      free_note: "flash tier",
      method: "api-key",
      category: "api-key",
      signup_status: "keyed",
    });
    expect(byId["opencode-go"]).toMatchObject({
      enabled: false,
      key_present: false,
      gateway: "unknown",
      method: "api-key",
      free_models: 0,
    });
    expect(byId.codex).toMatchObject({
      category: "oauth",
      method: "callback",
      credential_env: "",
      key_present: false,
      enabled: false,
    });
    expect(byId.freeinference.gateway).toBe("backoff");
    expect(byId.dahl.gateway).toBe("down");
    expect(rows.map((r) => r.id)).toEqual([...rows.map((r) => r.id)].sort());
  });

  it("marks every gateway bucket unknown when the gateway is unreachable", () => {
    const rows = buildProviderRows({ ...facts(), buckets: null });
    expect(rows.every((r) => r.gateway === "unknown")).toBe(true);
  });

  it("lowercases ids and never invents providers", () => {
    const rows = buildProviderRows({ ...facts(), catalog: [], oauth: [] });
    expect(rows.map((r) => r.id).sort()).toEqual(["glm", "opencode-go"]);
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
