import { describe, expect, it } from "vitest";
import {
  ENV_SCHEMA,
  REQUIRED,
  SETTINGS_SCHEMA,
  diffFingerprints,
  fingerprintConfig,
  getPath,
  provenanceEnvPairs,
  validateConfig,
  validateEnv,
  validateSettings,
} from "./config-schema.mjs";

const BASE_ENV = new Map([
  ["OFFICINA_PERMISSIONS_MODE", "yolo"],
  ["OFFICINA_CADUCEUS", "1"],
  ["OFFICINA_OVERFLOW", "1"],
  ["OFFICINA_OVERFLOW_CLOUD", "free"],
  ["OFFICINA_OVERFLOW_FAILOVER", "1"],
  ["OFFICINA_ROUTE_MODE", "auto"],
  ["OFFICINA_ROUTE_THRESHOLD", "0.5"],
  ["OFFICINA_CACHE_MODE", "auto"],
  ["LITTLE_CODER_CLEAR_KEEP", "8"],
]);

const BASE_SETTINGS = {
  defaultProvider: "officina",
  defaultModel: "lapis",
  compaction: {
    modelOverrides: {
      "llamacpp/Lapis Occultus": { reserveTokens: 65536, keepRecentTokens: 24000 },
      "officina/lapis": { reserveTokens: 65536, keepRecentTokens: 24000 },
    },
  },
};

describe("validateEnv", () => {
  it("passes the blessed shape", () => {
    const r = validateEnv(BASE_ENV);
    expect(r.ok).toBe(true);
    expect(r.errors).toEqual([]);
  });

  it("fails when a required key is missing", () => {
    const env = new Map(BASE_ENV);
    env.delete("OFFICINA_OVERFLOW_CLOUD");
    const r = validateEnv(env);
    expect(r.ok).toBe(false);
    expect(r.errors.map((e) => e.key)).toContain("OFFICINA_OVERFLOW_CLOUD");
    // The failure mode is part of the output, not just the key name.
    expect(r.errors.find((e) => e.key === "OFFICINA_OVERFLOW_CLOUD").why).toMatch(/candidate/);
  });

  it("treats an empty value as missing", () => {
    const env = new Map(BASE_ENV);
    env.set("OFFICINA_OVERFLOW", "");
    const r = validateEnv(env);
    expect(r.ok).toBe(false);
    expect(r.errors.map((e) => e.key)).toContain("OFFICINA_OVERFLOW");
  });

  it("warns (not errors) for expected keys", () => {
    const env = new Map(BASE_ENV);
    env.delete("LITTLE_CODER_CLEAR_KEEP");
    const r = validateEnv(env);
    expect(r.ok).toBe(true);
    expect(r.warnings.map((w) => w.key)).toContain("LITTLE_CODER_CLEAR_KEEP");
  });

  it("accepts a plain object as well as a Map", () => {
    expect(validateEnv(Object.fromEntries(BASE_ENV)).ok).toBe(true);
  });
});

describe("validateSettings", () => {
  it("passes the blessed shape", () => {
    expect(validateSettings(BASE_SETTINGS).ok).toBe(true);
  });

  it("fails when the default model is gone", () => {
    const s = { ...BASE_SETTINGS };
    delete s.defaultModel;
    const r = validateSettings(s);
    expect(r.ok).toBe(false);
    expect(r.errors.map((e) => e.key)).toContain("defaultModel");
  });

  it("warns when compaction reserves are missing at depth", () => {
    const s = {
      ...BASE_SETTINGS,
      compaction: { modelOverrides: {} },
    };
    const r = validateSettings(s);
    expect(r.ok).toBe(true);
    expect(r.warnings.length).toBe(4);
    expect(r.warnings[0].why).toMatch(/compaction|overflow/);
  });
});

describe("validateConfig", () => {
  it("merges env and settings findings", () => {
    const env = new Map(BASE_ENV);
    env.delete("OFFICINA_ROUTE_MODE");
    const r = validateConfig({ env, settings: {} });
    expect(r.ok).toBe(false);
    expect(r.errors.map((e) => e.key)).toEqual(
      expect.arrayContaining(["OFFICINA_ROUTE_MODE", "defaultProvider", "defaultModel"]),
    );
  });

  it("tolerates a missing env file", () => {
    expect(validateConfig({ env: null, settings: BASE_SETTINGS }).ok).toBe(true);
  });
});

describe("getPath", () => {
  it("walks nested objects", () => {
    expect(
      getPath(BASE_SETTINGS, ["compaction", "modelOverrides", "officina/lapis", "reserveTokens"]),
    ).toBe(65536);
  });

  it("returns undefined for missing or non-object segments", () => {
    expect(getPath(BASE_SETTINGS, ["nope", "deeper"])).toBeUndefined();
    expect(getPath(BASE_SETTINGS, ["defaultModel", "deeper"])).toBeUndefined();
    expect(getPath(null, ["a"])).toBeUndefined();
  });
});

describe("fingerprintConfig", () => {
  it("is stable across key insertion order", () => {
    const a = fingerprintConfig({ env: BASE_ENV, settings: BASE_SETTINGS });
    const shuffled = new Map([...BASE_ENV.entries()].reverse());
    const b = fingerprintConfig({ env: shuffled, settings: BASE_SETTINGS });
    expect(a.hash).toBe(b.hash);
  });

  it("changes when a provenance-bearing value changes", () => {
    const a = fingerprintConfig({ env: BASE_ENV, settings: BASE_SETTINGS });
    const env = new Map(BASE_ENV);
    env.set("OFFICINA_OVERFLOW_QUEUE_MS", "1500");
    expect(fingerprintConfig({ env, settings: BASE_SETTINGS }).hash).not.toBe(a.hash);
  });

  it("ignores non-provenance keys such as provider API keys", () => {
    const a = fingerprintConfig({ env: BASE_ENV, settings: BASE_SETTINGS });
    const env = new Map(BASE_ENV);
    env.set("AGENTROUTER_API_KEY", "sk-rotated");
    env.set("SOME_UNRELATED_KEY", "x");
    expect(fingerprintConfig({ env, settings: BASE_SETTINGS }).hash).toBe(a.hash);
  });

  it("never includes a secret in the pairs", () => {
    const env = new Map(BASE_ENV);
    env.set("OFFICINA_CADUCEUS_API_KEY", "sk-secret-value");
    const { pairs } = fingerprintConfig({ env, settings: BASE_SETTINGS });
    const blob = JSON.stringify(pairs);
    expect(blob).not.toContain("sk-secret-value");
    // Every fingerprinted key is declared in a schema — nothing is picked up
    // by scanning the live file, so a stray secret cannot leak in.
    const declared = new Set([
      ...ENV_SCHEMA.map((e) => e.key),
      ...SETTINGS_SCHEMA.map((e) => e.path.join(".")),
    ]);
    for (const [key] of pairs) expect(declared.has(key), key).toBe(true);
  });

  it("restricts env pairs to provenance-bearing schema entries", () => {
    const env = new Map(BASE_ENV);
    env.set("OFFICINA_PRAETOR", "0"); // declared, but provenance: false
    const keys = provenanceEnvPairs(env).map(([k]) => k);
    expect(keys).not.toContain("OFFICINA_PRAETOR");
    expect(keys).toContain("OFFICINA_OVERFLOW");
  });
});

describe("diffFingerprints", () => {
  const base = [
    ["A", "1"],
    ["B", "2"],
  ];

  it("reports a clean diff for identical pairs", () => {
    expect(diffFingerprints(base, base).clean).toBe(true);
  });

  it("detects added, removed and changed keys", () => {
    const next = [
      ["A", "9"],
      ["C", "3"],
    ];
    const d = diffFingerprints(base, next);
    expect(d.clean).toBe(false);
    expect(d.added).toEqual([["C", "3"]]);
    expect(d.removed).toEqual([["B", "2"]]);
    expect(d.changed).toEqual([{ key: "A", from: "1", to: "9" }]);
  });

  it("treats a dropped speed-bearing key as drift (the MTP failure mode)", () => {
    // Simulates `[spec]` disappearing from the live config.
    const before = [["OFFICINA_OVERFLOW_FAILOVER", "1"]];
    const after = [];
    const d = diffFingerprints(before, after);
    expect(d.clean).toBe(false);
    expect(d.removed).toEqual([["OFFICINA_OVERFLOW_FAILOVER", "1"]]);
  });
});

describe("schema hygiene", () => {
  it("gives every entry a why (the failure mode is the documentation)", () => {
    for (const entry of ENV_SCHEMA) {
      expect(entry.why, entry.key).toBeTruthy();
      expect(entry.level === REQUIRED || entry.level === "expected", entry.key).toBe(true);
    }
  });

  it("does not declare the same env key twice", () => {
    const keys = ENV_SCHEMA.map((e) => e.key);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
