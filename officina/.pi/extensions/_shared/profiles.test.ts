import { afterEach, describe, expect, it } from "vitest";
import { readProfile, sortIdsForProfile, writeProfile } from "./profiles.ts";

const file = "/tmp/officina-profile-test";
afterEach(() => {
  delete process.env.OFFICINA_ROUTING_PROFILE_FILE;
});

describe("routing profiles", () => {
  it("orders fast: deepseek 4.1 > deepseek > mimo > glm", () => {
    const ids = sortIdsForProfile(
      [
        { id: "opencode-go/glm-5.3-flash" },
        { id: "opencode-go/mimo-v2.6-flash" },
        { id: "opencode-go/deepseek-v4-flash" },
        { id: "opencode-go/deepseek-v4.1-flash" },
      ],
      "fast",
    ).map((x) => x.id);
    expect(ids).toEqual([
      "opencode-go/deepseek-v4.1-flash",
      "opencode-go/deepseek-v4-flash",
      "opencode-go/mimo-v2.6-flash",
      "opencode-go/glm-5.3-flash",
    ]);
  });

  it("orders deep: agentrouter first", () => {
    const ids = sortIdsForProfile(
      [{ id: "opencode-go/deepseek-v4.1-flash" }, { id: "agentrouter/claude-opus-5" }],
      "deep",
    ).map((x) => x.id);
    expect(ids[0]).toBe("agentrouter/claude-opus-5");
  });

  it("persists a profile", () => {
    process.env.OFFICINA_ROUTING_PROFILE_FILE = file;
    writeProfile("fast");
    expect(readProfile()).toBe("fast");
    writeProfile("local");
    expect(readProfile()).toBe("local");
  });
});
