// Voice suggestion rules (voice-suggestions stage 1, I3/I5).
import { describe, expect, it } from "vitest";
import {
  buildGenPrompt,
  cleanDraft,
  makeSuggestionId,
  resolveSuggestion,
  voiceBlock,
} from "./suggest.ts";

describe("resolveSuggestion", () => {
  it("heuristic wins whenever it fires (I3)", () => {
    const r = resolveSuggestion("▸ next: fix the failing gates — 1 gate failing (/verify full)", "try the thing");
    expect(r?.source).toBe("heuristic");
    expect(r?.text).toContain("fix the failing gates");
  });

  it("falls back to the cleaned LLM draft in silence", () => {
    const r = resolveSuggestion(null, "run the full verify suite");
    expect(r).toEqual({ text: "run the full verify suite", source: "llm" });
  });

  it("returns null when both are silent or the draft cleans to nothing", () => {
    expect(resolveSuggestion(null, null)).toBeNull();
    expect(resolveSuggestion(null, "```\n```")).toBeNull();
    expect(resolveSuggestion("", "   ")).toBeNull();
  });
});

describe("cleanDraft", () => {
  it("unwraps fences, prefixes, quotes and clamps length", () => {
    expect(cleanDraft("```text\nfix the flaky solver test\n```")).toBe("fix the flaky solver test");
    expect(cleanDraft("SUGGESTION: run /verify full")).toBe("run /verify full");
    expect(cleanDraft('"commit the work"')).toBe("commit the work");
    expect(cleanDraft("first line\nsecond line")).toBe("first line");
    const long = cleanDraft("x".repeat(400))!;
    expect(long.length).toBe(240);
    expect(long.endsWith("…")).toBe(true);
    expect(cleanDraft("")).toBeNull();
  });
});

describe("voiceBlock", () => {
  it("injects the profile or an honest fallback", () => {
    expect(voiceBlock("terse, imperative, no articles")).toContain("terse, imperative");
    expect(voiceBlock(null)).toContain("plain, direct imperative");
    expect(voiceBlock("  ")).toContain("plain, direct imperative");
  });
});

describe("buildGenPrompt", () => {
  it("carries signals and the single-line contract", () => {
    const p = buildGenPrompt(
      { verifyKnown: true, verifyOk: false, dirtyFiles: 3, lastAssistantTail: "done with the cache layer" },
      "terse, imperative",
    );
    expect(p).toContain("verify gates: FAILING");
    expect(p).toContain("3 changed file(s)");
    expect(p).toContain("done with the cache layer");
    expect(p).toContain("≤ 240 chars");
    expect(p).toContain("terse, imperative");
    expect(buildGenPrompt({}, null)).toContain("Signals:");
  });
});

describe("makeSuggestionId", () => {
  it("is unique enough per call", () => {
    const a = makeSuggestionId();
    const b = makeSuggestionId();
    expect(a).not.toBe(b);
    expect(a.startsWith("sug-")).toBe(true);
  });
});
