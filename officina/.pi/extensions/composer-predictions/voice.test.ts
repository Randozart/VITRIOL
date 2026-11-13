// Voice learning rules (voice-suggestions stage 3, I4: safe-only distill).
import { describe, expect, it } from "vitest";
import {
  acceptanceBlock,
  buildDistillPrompt,
  DISTILL_EVERY_PROMPTS,
  parseProfile,
  readProfile,
  redactSecrets,
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

describe("redactSecrets (stage 4, defense-in-depth)", () => {
  it("scrubs key-shaped strings from generated text", () => {
    // Fixtures are deliberately SHORT fakes: long enough for the
    // redaction regexes (>=8 after the prefix) but under the publish
    // canary's 20-char key threshold, so the release gate stays green.
    const dirty = [
      "use sk-FAKEKEY12345 for billing",
      "token ghp_FAKE1234567890123 in env",
      "fine-grain github_pat_FAKE12345678901234567890",
      "aws AKIAIOSFODNN7EXAMPLE key",
      "slack xoxb-FAKE12345678",
      "send Authorization: Bearer eyJa9.8sKm7 secret",
      "jwt eyJFAKEHEADER123456789012345.FAKEPAYLOAD1234567890",
      "digest deadbeefdeadbeefdeadbeefdeadbeef",
    ].join("\n");
    const clean = redactSecrets(dirty);
    expect(clean).not.toContain("sk-FAKEKEY12345");
    expect(clean).not.toContain("ghp_FAKE1234567890123");
    expect(clean).not.toContain("github_pat_FAKE12345678901234567890");
    expect(clean).not.toContain("AKIAIOSFODNN7EXAMPLE");
    expect(clean).not.toContain("xoxb-FAKE12345678");
    expect(clean).not.toContain("Bearer eyJa9");
    expect(clean).not.toContain("eyJFAKEHEADER123456789012345");
    expect(clean).not.toContain("deadbeefdeadbeefdeadbeefdeadbeef");
    expect(clean).toContain("use sk-REDACTED for billing");
    expect(redactSecrets("normal prose with the word sky and skunk")).toBe(
      "normal prose with the word sky and skunk",
    );
  });
});
