// Voice suggestion rules (voice-suggestions stage 1, I3/I5).
import { describe, expect, it } from "vitest";
import {
  buildGenPrompt,
  cleanDraft,
  makeSuggestionId,
  resolveSuggestion,
  suggestionAllowed,
  voiceBlock,
} from "./suggest.ts";

describe("resolveSuggestion", () => {
  it("heuristic wins whenever it is task-shaped (I3)", () => {
    const r = resolveSuggestion(
      "▸ next: wire the Esc handler",
      "try the thing",
    );
    expect(r?.source).toBe("heuristic");
    expect(r?.text).toContain("wire the Esc handler");
  });

  it("rejects instruction echoes from either side (N3)", () => {
    // The two lines the operator actually complained about.
    expect(
      resolveSuggestion("▸ next: commit the work (95 changed files)", null),
    ).toBeNull();
    expect(
      resolveSuggestion(
        "▸ next: fix the failing gates — 1 gate failing (/verify full)",
        null,
      ),
    ).toBeNull();
    expect(resolveSuggestion(null, "GRIND: DONE")).toBeNull();
    expect(
      resolveSuggestion(null, "the AGENTS.md instructions require a commit"),
    ).toBeNull();
    expect(resolveSuggestion(null, "Praetor wants intent comments")).toBeNull();
    // A rejected heuristic does not poison a good draft.
    expect(
      resolveSuggestion(
        "▸ next: commit the work (1 changed file)",
        "finish the audit table",
      ),
    ).toEqual({ text: "finish the audit table", source: "llm" });
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

describe("suggestionAllowed", () => {
  it("accepts work-shaped text and rejects doctrine", () => {
    expect(suggestionAllowed("check the KV restore path")).toBe(true);
    expect(suggestionAllowed("▸ next: wire the Esc handler")).toBe(true);
    expect(suggestionAllowed("")).toBe(false);
    expect(suggestionAllowed("  ")).toBe(false);
    expect(suggestionAllowed("ab")).toBe(false);
    expect(suggestionAllowed("commit the work (3 files)")).toBe(false);
    expect(suggestionAllowed("Stage 2: bound the blockers")).toBe(false);
    expect(suggestionAllowed("▸ next:")).toBe(false);
  });
});

describe("cleanDraft", () => {
  it("unwraps fences, prefixes, quotes and clamps length", () => {
    expect(cleanDraft("```text\nfix the flaky solver test\n```")).toBe(
      "fix the flaky solver test",
    );
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
    expect(voiceBlock("terse, imperative, no articles")).toContain(
      "terse, imperative",
    );
    expect(voiceBlock(null)).toContain("plain, direct imperative");
    expect(voiceBlock("  ")).toContain("plain, direct imperative");
  });
});

describe("buildGenPrompt", () => {
  it("carries signals and the single-line contract", () => {
    const p = buildGenPrompt(
      {
        verifyKnown: true,
        verifyOk: false,
        dirtyFiles: 3,
        lastAssistantTail: "done with the cache layer",
      },
      "terse, imperative",
    );
    expect(p).toContain("verify gates: FAILING");
    expect(p).toContain("3 changed file(s)");
    expect(p).toContain("done with the cache layer");
    expect(p).toContain("≤ 240 chars");
    expect(p).toContain("terse, imperative");
    expect(buildGenPrompt({}, null)).toContain("Signals:");
  });

  it("puts the request-aligned signals first and forbids rulebook echoes", () => {
    const p = buildGenPrompt(
      {
        taskTail: "wire the Esc handler",
        userPrompt: "esc doesn't properly end the work",
        lastAssistantTail: "stopped the queue",
        verifyKnown: true,
        verifyOk: true,
        dirtyFiles: 2,
      },
      null,
    );
    const task = p.indexOf("task in progress: wire the Esc handler");
    const request = p.indexOf(
      "operator's last request: esc doesn't properly end the work",
    );
    const tail = p.indexOf("assistant's last reply ended with");
    const hygiene = p.indexOf("hygiene only:");
    expect(task).toBeGreaterThan(-1);
    expect(task).toBeLessThan(request);
    expect(request).toBeLessThan(tail);
    expect(tail).toBeLessThan(hygiene);
    expect(p).toContain("Never restate harness rules");
    expect(p).toContain("'commit the work'");
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
