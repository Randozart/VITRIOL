import { describe, expect, it } from "vitest";
import {
  isPraetorSourceFile,
  parsePraetorJson,
  praetorConfig,
  renderPraetorDiagnostics,
} from "./praetor.ts";

describe("praetorConfig", () => {
  it("is off by default", () => {
    expect(praetorConfig({}).enabled).toBe(false);
  });
  it("arms from either env alias and honours the kill switch", () => {
    expect(praetorConfig({ OFFICINA_PRAETOR: "1" }).enabled).toBe(true);
    expect(praetorConfig({ TRIS_DIAG_PRAETOR: "1" }).enabled).toBe(true);
    expect(praetorConfig({ OFFICINA_PRAETOR: "1", OFFICINA_NO_PRAETOR: "1" }).enabled).toBe(false);
  });
  it("clamps budgets", () => {
    expect(praetorConfig({ OFFICINA_PRAETOR: "1", OFFICINA_PRAETOR_BUDGET: "1" }).budgetChars).toBe(100);
    expect(praetorConfig({ OFFICINA_PRAETOR: "1", OFFICINA_PRAETOR_BUDGET: "99999" }).budgetChars).toBe(4000);
  });
});

describe("parsePraetorJson", () => {
  it("extracts failures", () => {
    const diags = parsePraetorJson(
      JSON.stringify({
        failures: [{ file: "src/a.rs", line: 12, source: "complexity", message: "too complex" }],
      }),
    );
    expect(diags).toEqual([
      { file: "src/a.rs", line: 12, source: "complexity", message: "too complex" },
    ]);
  });
  it("returns empty for malformed output", () => {
    expect(parsePraetorJson("not json")).toEqual([]);
    expect(parsePraetorJson('{"failures":[]}')).toEqual([]);
  });
});

describe("renderPraetorDiagnostics", () => {
  it("renders a bounded advisory tail", () => {
    const text = renderPraetorDiagnostics(
      Array.from({ length: 20 }, (_, i) => ({
        file: `f${i}.rs`,
        line: i,
        source: "complexity",
        message: "x".repeat(50),
      })),
      300,
    );
    expect(text.length).toBeLessThanOrEqual(300);
    expect(text).toContain("praetor:");
    expect(text).toContain("advisory");
  });
});

describe("isPraetorSourceFile", () => {
  it("accepts source extensions only", () => {
    expect(isPraetorSourceFile("a.rs")).toBe(true);
    expect(isPraetorSourceFile("a.cpp")).toBe(true);
    expect(isPraetorSourceFile("a.md")).toBe(false);
  });
});
