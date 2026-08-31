import { describe, expect, it } from "vitest";
import { contractConfig, contractKindFor, duplicateKeys, runExternalContract, validateJson } from "./contract.ts";

describe("contractKindFor", () => {
  const cfg = contractConfig({ OFFICINA_CONTRACT_TOML: "1", OFFICINA_CONTRACT_YAML: "1" } as NodeJS.ProcessEnv);
  it("routes json/toml/yaml", () => {
    expect(contractKindFor("a.json", cfg)).toBe("json");
    expect(contractKindFor("b.toml", cfg)).toBe("toml");
    expect(contractKindFor("c.yml", cfg)).toBe("yaml");
    expect(contractKindFor("d.yaml", cfg)).toBe("yaml");
  });
  it("toml/yaml off by default", () => {
    const off = contractConfig({} as NodeJS.ProcessEnv);
    expect(contractKindFor("b.toml", off)).toBeNull();
    expect(contractKindFor("c.yml", off)).toBeNull();
    expect(contractKindFor("a.json", off)).toBe("json");
  });
});

describe("validateJson", () => {
  it("accepts clean JSON", () => {
    expect(validateJson('{"a":1,"b":[2,3]}')).toBeNull();
  });
  it("rejects syntax errors", () => {
    expect(validateJson('{"a":1,]')).toContain("invalid JSON");
  });
  it("detects duplicate keys", () => {
    const err = validateJson('{"name":"a","nested":{"name":"x","name":"y"}}');
    expect(err).toContain("duplicate JSON key");
    expect(err).toContain("name");
  });
  it("does not treat array strings as keys", () => {
    expect(validateJson('{"a":["x","x","x"]}')).toBeNull();
  });
});

describe("runExternalContract", () => {
  it("returns null on success", () => {
    expect(runExternalContract("toml", "f.toml", () => ({ ok: true, output: "" }))).toBeNull();
  });
  it("returns a capped error on failure", () => {
    const err = runExternalContract("yaml", "f.yml", () => ({ ok: false, output: "boom" }));
    expect(err).toContain("invalid YAML");
    expect(err).toContain("boom");
  });
});
