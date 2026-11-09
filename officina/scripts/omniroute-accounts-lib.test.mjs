import { describe, expect, it } from "vitest";
import {
  buildImportPayload,
  catalogMap,
  mapOpenCodeAuth,
  maskSecret,
  normalizeAccountsConfig,
  parseCliJson,
  selectOptionalFreeProviders,
  stripAnsi,
  summarizeImportResults,
} from "./omniroute-accounts-lib.mjs";

const catalog = catalogMap({
  providers: [
    { id: "opencode-go", category: "api-key" },
    { id: "openai", category: "api-key" },
    { id: "zai", category: "api-key" },
    { id: "glm", category: "api-key" },
    { id: "naga-ac", category: "api-key" },
    { id: "claude", category: "oauth" },
  ],
});

const config = (accounts, oauth = []) => ({ accounts, oauth });

describe("parseCliJson", () => {
  it("strips ANSI prelude before JSON", () => {
    const raw = "\x1b[2mloaded env\x1b[0m\n{\"providers\":[]}\n";
    expect(parseCliJson(raw)).toEqual({ providers: [] });
  });

  it("handles array output", () => {
    expect(parseCliJson("noise\n[1,2]\n")).toEqual([1, 2]);
  });

  it("throws when no JSON exists", () => {
    expect(() => parseCliJson("nothing here")).toThrow();
  });

  it("stripAnsi leaves plain text intact", () => {
    expect(stripAnsi("plain")).toBe("plain");
  });
});

describe("normalizeAccountsConfig", () => {
  it("accepts object form", () => {
    const result = normalizeAccountsConfig(config([{ provider: "openai" }], ["codex"]));
    expect(result.accounts).toHaveLength(1);
    expect(result.oauth).toEqual(["codex"]);
    expect(result.errors).toEqual([]);
  });

  it("accepts array form", () => {
    const result = normalizeAccountsConfig([{ provider: "openai" }]);
    expect(result.accounts).toHaveLength(1);
    expect(result.oauth).toEqual([]);
  });

  it("reports malformed config", () => {
    expect(normalizeAccountsConfig({ nope: true }).errors).toHaveLength(1);
  });
});

describe("buildImportPayload", () => {
  it("resolves credential env references", () => {
    const payload = buildImportPayload(
      config([
        {
          provider: "opencode-go",
          name: "opencode-go",
          credential_env: "OPENCODE_API_KEY",
          default_model: "gpt-6-luna",
          priority: 2,
        },
      ]),
      catalog,
      { OPENCODE_API_KEY: "secret-key" },
    );
    expect(payload.errors).toEqual([]);
    expect(payload.entries).toEqual([
      {
        provider: "opencode-go",
        name: "opencode-go",
        apiKey: "secret-key",
        defaultModel: "gpt-6-luna",
        priority: 2,
      },
    ]);
  });

  it("skips missing env vars by default", () => {
    const payload = buildImportPayload(
      config([{ provider: "openai", credential_env: "OPENAI_API_KEY" }]),
      catalog,
      {},
    );
    expect(payload.entries).toEqual([]);
    expect(payload.skipped[0].reason).toBe("missing_env:OPENAI_API_KEY");
  });

  it("turns missing env into an error in strict mode", () => {
    const payload = buildImportPayload(
      config([{ provider: "openai", credential_env: "OPENAI_API_KEY" }]),
      catalog,
      {},
      { strict: true },
    );
    expect(payload.errors[0].reason).toBe("missing_env:OPENAI_API_KEY");
  });

  it("rejects unknown providers", () => {
    const payload = buildImportPayload(
      config([{ provider: "nope", credential_env: "NOPE_KEY" }]),
      catalog,
      { NOPE_KEY: "x" },
    );
    expect(payload.errors[0].reason).toBe("unknown_provider");
  });

  it("rejects duplicate provider+name identities", () => {
    const payload = buildImportPayload(
      config([
        { provider: "openai", name: "main", credential_env: "A" },
        { provider: "openai", name: "main", credential_env: "B" },
      ]),
      catalog,
      { A: "one", B: "two" },
    );
    expect(payload.errors[0].reason).toBe("duplicate_identity");
  });

  it("supports no-credential entries", () => {
    const payload = buildImportPayload(
      config([{ provider: "openai", allow_no_credential: true }]),
      catalog,
      {},
    );
    expect(payload.entries[0]).toMatchObject({
      provider: "openai",
      name: "openai",
      allowNoCredential: true,
    });
  });

  it("forbids literal credentials in config", () => {
    const payload = buildImportPayload(
      config([{ provider: "openai", credential: "literal-secret" }]),
      catalog,
      {},
    );
    expect(payload.errors[0].reason).toBe("literal_credentials_forbidden");
  });

  it("filters with --only", () => {
    const result = buildImportPayload(
      config([
        { provider: "openai", credential_env: "OPENAI_API_KEY" },
        { provider: "opencode-go", credential_env: "OPENCODE_API_KEY" },
      ]),
      catalog,
      { OPENAI_API_KEY: "a", OPENCODE_API_KEY: "b" },
      { only: "opencode-go" },
    );
    expect(result.entries.map((entry) => entry.provider)).toEqual(["opencode-go"]);
    expect(result.skipped[0].reason).toBe("filtered");
  });

  it("passes provider-specific data through", () => {
    const payload = buildImportPayload(
      config([
        {
          provider: "openai",
          credential_env: "OPENAI_API_KEY",
          provider_specific_data: { region: "eu" },
        },
      ]),
      catalog,
      { OPENAI_API_KEY: "key" },
    );
    expect(payload.entries[0].providerSpecificData).toEqual({ region: "eu" });
  });

  it("rejects invalid priority and provider data", () => {
    const badPriority = buildImportPayload(
      config([{ provider: "openai", credential_env: "OPENAI_API_KEY", priority: 0 }]),
      catalog,
      { OPENAI_API_KEY: "key" },
    );
    expect(badPriority.errors[0].reason).toBe("invalid_priority");

    const badData = buildImportPayload(
      config([{ provider: "openai", credential_env: "OPENAI_API_KEY", provider_specific_data: [] }]),
      catalog,
      { OPENAI_API_KEY: "key" },
    );
    expect(badData.errors[0].reason).toBe("invalid_provider_specific_data");
  });
});

describe("summarizeImportResults", () => {
  it("counts import statuses", () => {
    const summary = summarizeImportResults([
      { status: "created" },
      { status: "skipped_existing" },
      { ok: false, status: "error", provider: "x", name: "y", error: "bad key" },
    ]);
    expect(summary.counts).toEqual({ created: 1, skipped_existing: 1, failed: 1, other: 0 });
    expect(summary.failed[0].error).toBe("bad key");
  });
});

describe("maskSecret", () => {
  it("never prints a full secret", () => {
    expect(maskSecret("sk-abcd1234")).toBe("...1234");
    expect(maskSecret("abc")).toBe("***");
  });
});


describe("mapOpenCodeAuth", () => {
  it("maps existing OpenCode API credentials", () => {
    const result = mapOpenCodeAuth(
      {
        "opencode-go": { type: "api", key: "go-key" },
        zai: { type: "api", key: "zai-key" },
        "zai-coding-plan": { type: "api", key: "coding-key" },
      },
      catalog,
    );
    expect(result.errors).toEqual([]);
    expect(result.entries).toEqual([
      { provider: "opencode-go", name: "opencode-go", apiKey: "go-key" },
      { provider: "zai", name: "zai", apiKey: "zai-key" },
      { provider: "glm", name: "glm-coding", apiKey: "coding-key" },
    ]);
  });

  it("skips missing, non-api, and already configured credentials", () => {
    const result = mapOpenCodeAuth(
      { "opencode-go": { type: "oauth" }, zai: { type: "api", key: "" } },
      catalog,
      new Set(["opencode-go\0opencode-go"]),
    );
    expect(result.entries).toEqual([]);
    expect(result.skipped.map((item) => item.reason).sort()).toEqual([
      "not_present",
      "unsupported_type",
      "empty_key",
    ].sort());
  });
});

describe("selectOptionalFreeProviders", () => {
  it("has no optional free providers by default", () => {
    const result = selectOptionalFreeProviders(catalog, new Set());
    expect(result.entries).toEqual([]);
    expect(result.errors).toEqual([]);
  });
});
