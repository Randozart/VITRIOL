// PROVENANCE: pattern from llama.cpp HTTP server.
// Contract tests for the hang-watchdog decision shim's pure pieces.
//
// The shim itself (decision-shim.cjs) starts an HTTP server on import, so the
// two functions whose contract matters are mirrored here. They are the parts
// the watchdog gate depends on: how a model's one-word answer becomes a
// structured verdict, and the shape/coverage of the response object.
import { describe, expect, it } from "vitest";

/** Mirror of parseChoice() in decision-shim.cjs. */
function parseChoice(text) {
  const trimmed = text
    .replace(/["'\n\r*`]/g, "")
    .trim()
    .toUpperCase();
  if (trimmed === "ALIVE") return { choice: "ALIVE", confidence: 0.95 };
  if (trimmed === "HUNG") return { choice: "HUNG", confidence: 0.95 };
  if (trimmed === "UNKNOWN") return { choice: "UNKNOWN", confidence: 0.95 };
  return { choice: "UNKNOWN", confidence: 0.3 };
}

function clamp(v) {
  return Number.isNaN(v) ? 0.5 : Math.max(0, Math.min(1, v));
}

/** Mirror of buildResponse() in decision-shim.cjs. */
function buildResponse(choice, confidence) {
  const answered = choice === "ALIVE" || choice === "HUNG";
  return {
    usage: {
      truncated: false,
      state_tokens_dropped: 0,
      truncated_questions: [],
      options: { liveness: { total: 3, distinct: 3 } },
    },
    answers: {
      liveness: {
        type: "choice",
        choice,
        answer_confidence: clamp(confidence),
        action: { act_probability: answered ? 0.99 : 0.1 },
        abstention: answered ? "passed" : "abstained",
        low_confidence: clamp(confidence) < 0.85,
      },
    },
  };
}

describe("parseChoice", () => {
  it("returns ALIVE for the bare word", () => {
    expect(parseChoice("ALIVE")).toEqual({ choice: "ALIVE", confidence: 0.95 });
  });

  it("returns HUNG for the bare word", () => {
    expect(parseChoice("HUNG")).toEqual({ choice: "HUNG", confidence: 0.95 });
  });

  it("returns UNKNOWN for the bare word", () => {
    expect(parseChoice("UNKNOWN")).toEqual({
      choice: "UNKNOWN",
      confidence: 0.95,
    });
  });

  it("degrades unknown text to a low-confidence UNKNOWN", () => {
    expect(parseChoice("maybe")).toEqual({
      choice: "UNKNOWN",
      confidence: 0.3,
    });
  });

  it("strips markdown fences and whitespace", () => {
    expect(parseChoice("```ALIVE```")).toEqual({
      choice: "ALIVE",
      confidence: 0.95,
    });
  });

  it("strips bold markers and newlines", () => {
    expect(parseChoice("**HUNG**\n")).toEqual({
      choice: "HUNG",
      confidence: 0.95,
    });
  });
});

describe("buildResponse", () => {
  it("reports a clear ALIVE as answered", () => {
    const r = buildResponse("ALIVE", 0.95);
    expect(r.answers.liveness.choice).toBe("ALIVE");
    expect(r.answers.liveness.action.act_probability).toBe(0.99);
    expect(r.answers.liveness.abstention).toBe("passed");
    expect(r.answers.liveness.low_confidence).toBe(false);
  });

  it("flags low confidence on HUNG", () => {
    const r = buildResponse("HUNG", 0.7);
    expect(r.answers.liveness.low_confidence).toBe(true);
  });

  it("marks UNKNOWN as abstained and inert", () => {
    const r = buildResponse("UNKNOWN", 0.3);
    expect(r.answers.liveness.abstention).toBe("abstained");
    expect(r.answers.liveness.action.act_probability).toBe(0.1);
  });

  it("clamps confidence into [0,1]", () => {
    expect(buildResponse("ALIVE", 1.5).answers.liveness.answer_confidence).toBe(
      1,
    );
    expect(
      buildResponse("ALIVE", -0.5).answers.liveness.answer_confidence,
    ).toBe(0);
  });

  it("defaults a NaN confidence to 0.5", () => {
    expect(buildResponse("ALIVE", NaN).answers.liveness.answer_confidence).toBe(
      0.5,
    );
  });

  it("declares truthful coverage (the model has exactly three choices)", () => {
    const r = buildResponse("ALIVE", 0.9);
    expect(r.usage.options.liveness).toEqual({ total: 3, distinct: 3 });
    expect(r.usage.truncated).toBe(false);
    expect(r.usage.state_tokens_dropped).toBe(0);
  });
});
