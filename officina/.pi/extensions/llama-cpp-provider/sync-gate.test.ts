import { describe, expect, it } from "vitest";
import { shouldArmEngineSync } from "./config.ts";

describe("shouldArmEngineSync", () => {
  it("arms when no model is selected yet", () => {
    expect(shouldArmEngineSync(undefined)).toBe(true);
    expect(shouldArmEngineSync("")).toBe(true);
  });

  it("arms for the local engine", () => {
    expect(shouldArmEngineSync("llamacpp")).toBe(true);
  });

  it("does not arm for explicit non-local providers", () => {
    expect(shouldArmEngineSync("caduceus")).toBe(false);
    expect(shouldArmEngineSync("opencode-go")).toBe(false);
    expect(shouldArmEngineSync("anthropic")).toBe(false);
    expect(shouldArmEngineSync("ollama")).toBe(false);
  });
});
