// Voice learning rules (voice-suggestions stage 3, I4: safe-only distill).
import { describe, expect, it } from "vitest";
import {
  acceptanceBlock,
  buildDistillPrompt,
  DISTILL_EVERY_PROMPTS,
  parseProfile,
  readProfile,
  shouldDistill,
  type VoiceProfile,
} from "./voice.ts";

const profile = (over: Partial<VoiceProfile> = {}): VoiceProfile => ({
  version: 1,
  updatedAt: new Date().toISOString(),
  profile: "terse, imperative",
  samples: 0,
  ...over,
});

describe("shouldDistill", () => {
  it("needs at least 5 safe prompts", () => {
    expect(shouldDistill(null, 4).ok).toBe(false);
    expect(shouldDistill(null, 5).ok).toBe(true);
  });

  it("is fresh for the day unless DISTILL_EVERY_PROMPTS new prompts arrived", () => {
    const p = profile({ samples: 10 });
    const sameDay = shouldDistill(p, 20);
    expect(sameDay.ok).toBe(false);
    const enough = shouldDistill(p, 10 + DISTILL_EVERY_PROMPTS);
    expect(enough.ok).toBe(true);
  });

  it("always distills on a new day with enough prompts", () => {
    const p = profile({ updatedAt: "1999-01-01T00:00:00.000Z", samples: 30 });
    expect(shouldDistill(p, 31).ok).toBe(true);
  });

  it("guards against corrupt profiles", () => {
    expect(shouldDistill({ version: 9 } as unknown as VoiceProfile, 10).ok).toBe(true);
  });
});

describe("parseProfile", () => {
  it("unwraps fences, clamps to 200 words, rejects empty", () => {
    expect(parseProfile("```\nterse imperative, no articles\n```")).toBe(
      "terse imperative, no articles",
    );
    expect(parseProfile("Profile: direct and terse")).toBe("direct and terse");
    const long = parseProfile(Array.from({ length: 300 }, (_, i) => `w${i}`).join(" "))!;
    expect(long.split(/\s+/)).toHaveLength(200);
    expect(parseProfile("   ")).toBeNull();
    expect(parseProfile(null)).toBeNull();
  });
});

describe("acceptanceBlock", () => {
  it("summarizes verdicts without any prompt content (I4)", () => {
    const block = acceptanceBlock([
      { verdict: "accepted", source: "llm" },
      { verdict: "accepted", source: "llm" },
      { verdict: "dismissed-typed", source: "heuristic" },
    ]);
    expect(block).toContain("2/3");
    expect(block).toContain("llm:2");
    expect(block.toLowerCase()).not.toContain("prompt");
  });
});

describe("buildDistillPrompt", () => {
  it("carries profile, safe prompts and accepted suggestions", () => {
    const p = buildDistillPrompt(
      profile(),
      ["fix the flaky solver test", "commit the cache layer"],
      ["run the full verify suite"],
    );
    expect(p).toContain("terse, imperative");
    expect(p).toContain("fix the flaky solver test");
    expect(p).toContain("run the full verify suite");
    expect(p).toContain("200 words");
    const fresh = buildDistillPrompt(null, [], []);
    expect(fresh).toContain("(none yet");
    expect(fresh).toContain("(none)");
  });
});

describe("readProfile", () => {
  it("rejects wrong versions and garbage", () => {
    expect(readProfile("/definitely/not/here.json")).toBeNull();
    expect(readProfile("/definitely/not/here.json")).toBeNull();
  });
});
