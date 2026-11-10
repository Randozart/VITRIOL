import { describe, expect, it } from "vitest";
import { extractCatalogModels, filterCatalogByProviders, freeToolCandidates, mapCatalogModels } from "./catalog.ts";

describe("extractCatalogModels", () => {
  it("accepts raw arrays", () => {
    expect(extractCatalogModels([{ id: "a" }])).toEqual([{ id: "a" }]);
  });
  it("accepts OpenAI data envelopes", () => {
    expect(extractCatalogModels({ data: [{ id: "a" }] })).toEqual([{ id: "a" }]);
  });
  it("accepts models envelopes", () => {
    expect(extractCatalogModels({ models: [{ id: "a" }] })).toEqual([{ id: "a" }]);
  });
  it("returns empty for unknown shapes", () => {
    expect(extractCatalogModels({ nope: true })).toEqual([]);
  });
});

describe("mapCatalogModels", () => {
  it("maps canonical ids and metadata", () => {
    const [model] = mapCatalogModels([
      {
        id: "opencode-go/gpt-6-luna",
        display_name: "GPT-6 Luna",
        context_length: 400000,
        max_output_tokens: 32768,
        capabilities: { reasoning: true, vision: true, tool_calling: true },
        pricing: { input: 1.25, output: 10, cache_read: 0.25, cache_write: 1.5 },
      },
    ]);

    expect(model).toMatchObject({
      id: "opencode-go/gpt-6-luna",
      name: "GPT-6 Luna",
      reasoning: true,
      input: ["text", "image"],
      contextWindow: 400000,
      maxTokens: 32768,
      cost: { input: 1.25, output: 10, cacheRead: 0.25, cacheWrite: 1.5 },
    });
  });

  it("falls back to safe defaults", () => {
    const [model] = mapCatalogModels([{ id: "opencode-go/small" }]);
    expect(model.contextWindow).toBe(200000);
    expect(model.maxTokens).toBe(16384);
    expect(model.reasoning).toBe(false);
    expect(model.input).toEqual(["text"]);
    expect(model.cost).toEqual({ input: 0, output: 0, cacheRead: 0, cacheWrite: 0 });
  });

  it("clamps max output to the context window", () => {
    const [model] = mapCatalogModels([
      { id: "x/m", context_length: 8192, max_output_tokens: 65536 },
    ]);
    expect(model.maxTokens).toBe(8192);
  });

  it("reads reasoning effort objects", () => {
    const [model] = mapCatalogModels([
      { id: "x/m", capabilities: { reasoning: { effort: ["low", "high"] } } },
    ]);
    expect(model.reasoning).toBe(true);
  });

  it("reads image modalities", () => {
    const [model] = mapCatalogModels([
      { id: "x/m", modalities: { input: ["text", "image"] } },
    ]);
    expect(model.input).toEqual(["text", "image"]);
  });

  it("filters non-chat specialty rows", () => {
    const models = mapCatalogModels([
      { id: "x/embed", type: "embedding" },
      { id: "x/image", type: "image" },
      { id: "x/chat", type: "chat" },
    ]);
    expect(models.map((m) => m.id)).toEqual(["x/chat"]);
  });

  it("filters catalogs without chat endpoints", () => {
    const models = mapCatalogModels([
      { id: "x/embed", supported_endpoints: ["embeddings"] },
      { id: "x/chat", supported_endpoints: ["chat"] },
    ]);
    expect(models.map((m) => m.id)).toEqual(["x/chat"]);
  });

  it("dedupes and sorts by id", () => {
    const models = mapCatalogModels([
      { id: "b/model" },
      { id: "a/model" },
      { id: "b/model", display_name: "duplicate" },
    ]);
    expect(models.map((m) => m.id)).toEqual(["a/model", "b/model"]);
    expect(models[1].name).toBe("model");
  });
});


describe("filterCatalogByProviders", () => {
  it("keeps models from active providers plus auto", () => {
    const raw = [
      { id: "ddgw/m", owned_by: "duckduckgo-web" },
      { id: "aug/m", owned_by: "auggie" },
      { id: "auto/m", owned_by: "auto" },
      { id: "unknown/m" },
    ];
    const filtered = filterCatalogByProviders(raw, new Set(["duckduckgo-web"]));
    expect(filtered.map((item) => (item as { id: string }).id)).toEqual([
      "ddgw/m",
      "auto/m",
      "unknown/m",
    ]);
  });

  it("returns the raw catalog when active providers are unknown", () => {
    const raw = [{ id: "x/m", owned_by: "x" }];
    expect(filterCatalogByProviders(raw, null)).toBe(raw);
  });

  it("returns empty when there are no active providers", () => {
    expect(filterCatalogByProviders([{ id: "x/m", owned_by: "x" }], new Set())).toEqual([]);
  });
});


describe("freeToolCandidates", () => {
  it("keeps only tool-capable zero-cost agent lanes, in owner priority", () => {
    const rows = [
      { id: "ddgw/claude-haiku-4-5", owned_by: "duckduckgo-web", pricing: { input: 0, output: 0 }, capabilities: { tool_calling: false }, context_length: 200000 },
      { id: "opencode-go/deepseek-v4.1-flash", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "opencode-go/deepseek-v4.1-flash-high", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "opencode-go/qwen3.8-flash", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "dahl/MiniMaxAI/MiniMax-M2.7", owned_by: "dahl", pricing: null, capabilities: { tool_calling: true }, context_length: 204800 },
      { id: "glm/glm-4.7-flash", owned_by: "glm", pricing: { input: 0, output: 0 }, capabilities: { tool_calling: true }, context_length: 200000 },
      { id: "glm/paid", owned_by: "glm", pricing: { input: 1, output: 2 }, capabilities: { tool_calling: true }, context_length: 200000 },
      { id: "zai/glm-5.3-flash", owned_by: "zai", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "cfp/openai/gpt-oss-120b", owned_by: "cloudflare-playground", pricing: null, capabilities: { tool_calling: true }, context_length: 400000 },
      { id: "aihorde/pic", owned_by: "aihorde", pricing: null, capabilities: { tool_calling: false }, context_length: 200000 },
    ];
    const ids = freeToolCandidates(rows).map((m) => m.id);
    expect(ids).toContain("opencode-go/deepseek-v4.1-flash");
    expect(ids).toContain("opencode-go/qwen3.8-flash");
    expect(ids).toContain("dahl/MiniMaxAI/MiniMax-M2.7");
    expect(ids).toContain("glm/glm-4.7-flash");
    expect(ids).toContain("cfp/openai/gpt-oss-120b");
    expect(ids).not.toContain("ddgw/claude-haiku-4-5");
    expect(ids).not.toContain("glm/paid");
    expect(ids).not.toContain("zai/glm-5.3-flash");
    expect(ids).not.toContain("opencode-go/deepseek-v4.1-flash-high");
    expect(ids.indexOf("opencode-go/deepseek-v4.1-flash")).toBeLessThan(
      ids.indexOf("dahl/MiniMaxAI/MiniMax-M2.7"),
    );
    expect(ids.indexOf("dahl/MiniMaxAI/MiniMax-M2.7")).toBeLessThan(
      ids.indexOf("glm/glm-4.7-flash"),
    );
  });
});


describe("free model preference", () => {
  it("prefers DeepSeek V4.1 Flash, then native DeepSeek, MiMo, GLM", () => {
    const rows = [
      { id: "opencode-go/glm-5.3-flash", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "opencode-go/mimo-v2.6-flash", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "opencode-go/deepseek-v4-flash", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
      { id: "opencode-go/deepseek-v4.1-flash", owned_by: "opencode-go", pricing: null, capabilities: { tool_calling: true }, context_length: 1000000 },
    ];
    const ids = freeToolCandidates(rows).map((m) => m.id);
    expect(ids).toEqual([
      "opencode-go/deepseek-v4.1-flash",
      "opencode-go/deepseek-v4-flash",
      "opencode-go/mimo-v2.6-flash",
      "opencode-go/glm-5.3-flash",
    ]);
  });

  it("includes zero-cost tool-capable lanes from signed-up free owners", () => {
    const rows = [
      { id: "agentrouter/gpt-5.6-sol", owned_by: "agentrouter", pricing: { input: 0, output: 0 }, capabilities: { tool_calling: true }, context_length: 1050000 },
      { id: "agnes/agnes-3.0-flash", owned_by: "agnes", pricing: { input: 0, output: 0 }, capabilities: { tool_calling: true }, context_length: 524288 },
      { id: "api-airforce/qwen/qwen3-32b", owned_by: "api-airforce", pricing: { input: 0, output: 0 }, capabilities: { tool_calling: true }, context_length: 1048576 },
      { id: "friendliai/meta-llama-3.1-70b-instruct", owned_by: "friendliai", pricing: { input: 0, output: 0 }, capabilities: { tool_calling: true }, context_length: 128000 },
      { id: "baseten/zai-org/GLM-5.3-Flash", owned_by: "baseten", pricing: null, capabilities: { tool_calling: true }, context_length: 1048576 },
    ];
    const ids = freeToolCandidates(rows).map((m) => m.id);
    expect(ids).toContain("agentrouter/gpt-5.6-sol");
    expect(ids).toContain("agnes/agnes-3.0-flash");
    expect(ids).toContain("api-airforce/qwen/qwen3-32b");
    expect(ids).toContain("friendliai/meta-llama-3.1-70b-instruct");
    expect(ids).not.toContain("baseten/zai-org/GLM-5.3-Flash");
  });
});
