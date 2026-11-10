import { describe, expect, it } from "vitest";
import {
  FIAT_LUX_ID,
  buildFiatLuxCoupling,
  isFreeModel,
  mergeFiatLuxIntoCouplings,
  membershipState,
} from "./fiat-lux-lib.mjs";

const CATALOG = [
  { id: "opencode-go/mimo-v2.6-flash-free", name: "MiMo V2.6 Flash Free", context_length: 131072, max_output_tokens: 16384 },
  { id: "opencode-go/ox-alpha-free", name: "ox-alpha (free)", context_length: 65536 },
  { id: "opencode-go/ox-alpha", name: "ox-alpha (paid)" },
  { id: "ddgw/gpt-5.4-mini", name: "GPT-5.4 Mini" },
  { id: "freebuff/some-model", name: "Some Model" },
  { id: "cloudflare-playground/gpt-oss-120b", name: "GPT-OSS 120B (free tier)", context_length: 131072 },
  null,
  42,
];

describe("isFreeModel", () => {
  it("detects free markers in id, name, root, display_name", () => {
    expect(isFreeModel(CATALOG[0])).toBe(true);
    expect(isFreeModel(CATALOG[1])).toBe(true);
    expect(isFreeModel(CATALOG[2])).toBe(false);
    expect(isFreeModel({ id: "x", root: "y-free" })).toBe(true);
    expect(isFreeModel({ id: "x", display_name: "$0 model" })).toBe(true);
    expect(isFreeModel(null)).toBe(false);
    expect(isFreeModel(42)).toBe(false);
  });
});

describe("buildFiatLuxCoupling", () => {
  it("builds a deterministic CouplingDef from free rows only", () => {
    const coupling = buildFiatLuxCoupling(CATALOG, "http://127.0.0.1:20128/v1/");
    expect(coupling.id).toBe(FIAT_LUX_ID);
    expect(coupling.baseUrl).toBe("http://127.0.0.1:20128/v1");
    expect(coupling.api).toBe("openai-completions");
    expect(coupling.models.map((m) => m.id)).toEqual([
      "cloudflare-playground/gpt-oss-120b",
      "opencode-go/mimo-v2.6-flash-free",
      "opencode-go/ox-alpha-free",
    ]);
    const mimo = coupling.models.find((m) => m.id === "opencode-go/mimo-v2.6-flash-free");
    expect(mimo.contextWindow).toBe(131072);
    expect(mimo.maxTokens).toBe(16384);
    const ox = coupling.models.find((m) => m.id === "opencode-go/ox-alpha-free");
    expect(ox.contextWindow).toBe(65536);
    expect(ox.maxTokens).toBe(8192);
  });

  it("returns null when no free models exist", () => {
    expect(buildFiatLuxCoupling([CATALOG[3]], "http://x")).toBeNull();
    expect(buildFiatLuxCoupling([], "http://x")).toBeNull();
  });
});

describe("mergeFiatLuxIntoCouplings", () => {
  it("adds the coupling when the file is empty", () => {
    const coupling = buildFiatLuxCoupling(CATALOG, "http://127.0.0.1:20128/v1");
    const { text, changed } = mergeFiatLuxIntoCouplings("", coupling);
    expect(changed).toBe(true);
    const parsed = JSON.parse(text);
    expect(parsed.couplings).toHaveLength(1);
    expect(parsed.couplings[0].id).toBe(FIAT_LUX_ID);
  });

  it("replaces a prior fiat-lux entry and preserves sibling couplings + unknown keys", () => {
    const prior = {
      _comment: "keep me",
      couplings: [
        { id: "ascensus", name: "Ascensus", baseUrl: "https://api.z.ai/x", api: "openai-completions", apiKeyEnv: "ZAI_API_KEY", models: [{ id: "glm-5.3-flash", name: "GLM", contextWindow: 200000, maxTokens: 8192 }] },
        { id: FIAT_LUX_ID, name: "old", baseUrl: "http://old", api: "openai-completions", models: [{ id: "stale", name: "stale", contextWindow: 1, maxTokens: 1 }] },
      ],
    };
    const coupling = buildFiatLuxCoupling(CATALOG, "http://127.0.0.1:20128/v1");
    const { text, changed } = mergeFiatLuxIntoCouplings(JSON.stringify(prior), coupling);
    expect(changed).toBe(true);
    const parsed = JSON.parse(text);
    expect(parsed._comment).toBe("keep me");
    expect(parsed.couplings.map((c) => c.id)).toEqual(["ascensus", FIAT_LUX_ID]);
    expect(parsed.couplings.find((c) => c.id === FIAT_LUX_ID).models).toHaveLength(3);
    expect(coupling.models).toHaveLength(3);
  });

  it("is idempotent when the coupling is unchanged", () => {
    const coupling = buildFiatLuxCoupling(CATALOG, "http://127.0.0.1:20128/v1");
    const first = mergeFiatLuxIntoCouplings("", coupling);
    const second = mergeFiatLuxIntoCouplings(first.text, coupling);
    expect(second.changed).toBe(false);
  });

  it("recovers from a rotten file", () => {
    const coupling = buildFiatLuxCoupling(CATALOG, "http://127.0.0.1:20128/v1");
    const { text } = mergeFiatLuxIntoCouplings("{ not json", coupling);
    expect(JSON.parse(text).couplings).toHaveLength(1);
  });
});

describe("membershipState", () => {
  it("classifies providers by effort ladder", () => {
    const ctx = {
      connectedProviders: new Set(["opencode-go"]),
      authProviders: new Set(["zai"]),
      oauthProviders: new Set(["claude-code"]),
    };
    expect(membershipState("opencode-go", ctx)).toBe("active");
    expect(membershipState("ZAI", ctx)).toBe("auth-reused");
    expect(membershipState("claude-code", ctx)).toBe("oauth");
    expect(membershipState("cloudflare-playground/gpt-oss", ctx)).toBe("api-key");
  });
});
