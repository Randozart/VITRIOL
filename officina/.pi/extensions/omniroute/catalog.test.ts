import { describe, expect, it } from "vitest";
import { extractCatalogModels, filterCatalogByProviders, mapCatalogModels } from "./catalog.ts";

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
