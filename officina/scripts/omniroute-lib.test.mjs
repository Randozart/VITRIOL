import { describe, expect, it } from "vitest";
import { parseEnvFile, upsertEnvFile } from "./omniroute-lib.mjs";

describe("parseEnvFile", () => {
  it("parses simple values and ignores comments", () => {
    const env = parseEnvFile("# note\nA=1\n\nB=two\n");
    expect(env.get("A")).toBe("1");
    expect(env.get("B")).toBe("two");
  });

  it("strips surrounding quotes", () => {
    const env = parseEnvFile('A="hello world"\nB=\'x y\'\n');
    expect(env.get("A")).toBe("hello world");
    expect(env.get("B")).toBe("x y");
  });
});

describe("upsertEnvFile", () => {
  it("replaces keys and preserves unrelated lines", () => {
    const next = upsertEnvFile("# keep\nA=old\nB=stay\n", { A: "new" });
    expect(next).toBe("# keep\nA=new\nB=stay\n");
  });

  it("appends missing keys", () => {
    const next = upsertEnvFile("A=1\n", { B: "2" });
    expect(next).toBe("A=1\nB=2\n");
  });

  it("quotes values with spaces", () => {
    const next = upsertEnvFile("", { A: "hello world" });
    expect(next).toBe('A="hello world"\n');
  });
});
