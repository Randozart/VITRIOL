import { describe, expect, it } from "vitest";
import { missingModels } from "./index.ts";

describe("opencode-go catalog completion", () => {
  it("adds only models absent from the built-in catalog", () => {
    const missing = missingModels(["glm-5.2", "kimi-k3"]);
    expect(missing.map((m) => m.id).sort()).toEqual(["hy4", "longcat-2.5-free"]);
  });

  it("does not duplicate ids a future pi catalog already ships", () => {
    expect(missingModels(["hy4", "longcat-2.5-free"])).toEqual([]);
  });

  it("uses the live OpenCode Go endpoint and chat API", () => {
    for (const model of missingModels([])) {
      expect(model.api).toBe("openai-completions");
      expect(model.baseUrl).toBe("https://opencode.ai/inference/go/openai/v1");
      expect(model.type).toBe("chat");
      expect(model.input).toContain("text");
    }
  });
});
