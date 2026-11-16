// Gemini Decision 1 mapping tests (2026-11-16). No network: the fetch is
// injected, and the shapes come from the documented Decisions API.
import { describe, expect, it } from "vitest";
import {
  askGeminiDecision,
  buildDecisionsRequest,
  mapDecisionsReply,
  type DecisionEvidence,
} from "./decision-gemini.ts";
import { REDACTED } from "./decision-redact.ts";

const EVIDENCE: DecisionEvidence = {
  command: "cargo build --release",
  elapsedS: 120,
  stalledS: 0,
  outputTail: "Compiling officina-tui v0.2.0",
};

const reply = (
  choice: string,
  probs: Record<string, number>,
  confidence?: number,
) => ({
  answers: {
    liveness: {
      choice,
      probabilities: probs,
      confidence: confidence ?? probs[choice],
    },
  },
  usage: { input_tokens: 300, output_tokens: 0 },
});

describe("buildDecisionsRequest", () => {
  it("asks one 3-way choice question over shared evidence", () => {
    const body = buildDecisionsRequest(EVIDENCE) as {
      model: string;
      input: string;
      questions: Record<
        string,
        { type: string; criteria: Record<string, string> }
      >;
    };
    expect(body.model).toBe("gemini-decision-1");
    expect(body.input).toContain("Command: cargo build --release");
    expect(body.input).toContain("Elapsed: 120s | Stalled: 0s");
    expect(body.questions.liveness.type).toBe("choice");
    expect(Object.keys(body.questions.liveness.criteria)).toEqual([
      "ALIVE",
      "HUNG",
      "UNKNOWN",
    ]);
  });

  it("redacts evidence before it leaves the box", () => {
    const body = buildDecisionsRequest({
      ...EVIDENCE,
      command: "GEMINI_API_KEY=AIza" + "z".repeat(35) + " cargo test",
    }) as { input: string };
    expect(body.input).not.toContain("AIza");
    expect(body.input).toContain(REDACTED);
  });
});

describe("mapDecisionsReply", () => {
  it("accepts a confident ALIVE and mirrors confidence into both scores", () => {
    const out = mapDecisionsReply(
      reply("ALIVE", { ALIVE: 0.93, HUNG: 0.05, UNKNOWN: 0.02 }),
      {
        floor: 0.6,
        margin: 0.15,
      },
    );
    expect(out.ok).toBe(true);
    if (!out.ok) return;
    const liveness = out.reply.answers!.liveness!;
    expect(liveness.choice).toBe("ALIVE");
    expect(liveness.answer_confidence).toBeCloseTo(0.93);
    // The gate needs BOTH scores above the floor: P(HUNG) here would fail it.
    expect(liveness.action!.act_probability).toBeCloseTo(0.93);
    expect(liveness.abstention).toBe("passed");
    expect(out.reply.usage!.options!.liveness).toEqual({
      total: 3,
      distinct: 3,
    });
  });

  it("abstains below the floor", () => {
    const out = mapDecisionsReply(
      reply("HUNG", { ALIVE: 0.5, HUNG: 0.45, UNKNOWN: 0.05 }),
      {
        floor: 0.6,
      },
    );
    expect(out.ok).toBe(true);
    if (!out.ok) return;
    expect(out.reply.answers!.liveness!.abstention).toBe("abstained");
    expect(out.reason).toContain("below floor");
  });

  it("abstains when the margin is too small even above the floor", () => {
    const out = mapDecisionsReply(
      reply("HUNG", { HUNG: 0.62, ALIVE: 0.55, UNKNOWN: 0.03 }),
      {
        floor: 0.6,
        margin: 0.15,
      },
    );
    expect(out.ok).toBe(true);
    if (!out.ok) return;
    expect(out.reply.answers!.liveness!.abstention).toBe("abstained");
    expect(out.reason).toContain("ambiguous");
  });

  it("treats the model's own UNKNOWN as an abstention", () => {
    const out = mapDecisionsReply(
      reply("UNKNOWN", { ALIVE: 0.3, HUNG: 0.2, UNKNOWN: 0.5 }),
      {
        floor: 0.4,
      },
    );
    expect(out.ok).toBe(true);
    if (!out.ok) return;
    expect(out.reply.answers!.liveness!.choice).toBe("UNKNOWN");
    expect(out.reply.answers!.liveness!.abstention).toBe("abstained");
  });

  it("reports a safety block instead of a verdict", () => {
    const out = mapDecisionsReply(
      { promptFeedback: { blockReason: "blocklist" } },
      { floor: 0.6 },
    );
    expect(out.ok).toBe(false);
    if (out.ok) return;
    expect(out.reason).toContain("safety block");
  });

  it("reports a reply with no answer", () => {
    const out = mapDecisionsReply({}, { floor: 0.6 });
    expect(out.ok).toBe(false);
    if (out.ok) return;
    expect(out.reason).toContain("no liveness answer");
  });
});

describe("askGeminiDecision", () => {
  const respond = (status: number, body: unknown) =>
    (async () =>
      new Response(typeof body === "string" ? body : JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      })) as unknown as typeof fetch;

  it("posts to the decisions endpoint with the api-key header", async () => {
    let seen:
      | { url: string; headers: Record<string, string>; body: unknown }
      | undefined;
    const fetchImpl = (async (url: string, init: RequestInit) => {
      seen = {
        url: String(url),
        headers: init.headers as Record<string, string>,
        body: JSON.parse(String(init.body)),
      };
      return new Response(
        JSON.stringify(
          reply("ALIVE", { ALIVE: 0.9, HUNG: 0.06, UNKNOWN: 0.04 }),
        ),
        { status: 200 },
      );
    }) as unknown as typeof fetch;

    const out = await askGeminiDecision(EVIDENCE, {
      key: "test-key",
      floor: 0.6,
      fetchImpl,
    });
    expect(out.ok).toBe(true);
    expect(seen!.url).toContain("/v1beta/decisions");
    expect(seen!.url).not.toContain("key=");
    expect(seen!.headers["x-goog-api-key"]).toBe("test-key");
    expect((seen!.body as { model: string }).model).toBe("gemini-decision-1");
  });

  it("refuses to call without a key", async () => {
    const out = await askGeminiDecision(EVIDENCE, { key: "  ", floor: 0.6 });
    expect(out.ok).toBe(false);
    if (out.ok) return;
    expect(out.reason).toContain("no gemini key");
  });

  it("maps 429 to rate limited and safety 400 to a block", async () => {
    const limited = await askGeminiDecision(EVIDENCE, {
      key: "k",
      floor: 0.6,
      fetchImpl: respond(429, { error: { code: "RESOURCE_EXHAUSTED" } }),
    });
    expect(limited.ok).toBe(false);
    if (!limited.ok) expect(limited.reason).toContain("rate limited");

    const blocked = await askGeminiDecision(EVIDENCE, {
      key: "k",
      floor: 0.6,
      fetchImpl: respond(400, { error: { code: "prohibited_content" } }),
    });
    expect(blocked.ok).toBe(false);
    if (!blocked.ok) expect(blocked.reason).toContain("safety block");
  });

  it("reports an unusable body and a timeout", async () => {
    const junk = await askGeminiDecision(EVIDENCE, {
      key: "k",
      floor: 0.6,
      fetchImpl: respond(200, "not json"),
    });
    expect(junk.ok).toBe(false);
    if (!junk.ok) expect(junk.reason).toContain("not JSON");

    const aborting = (async () => {
      const err = new Error("aborted");
      err.name = "AbortError";
      throw err;
    }) as unknown as typeof fetch;
    const timedOut = await askGeminiDecision(EVIDENCE, {
      key: "k",
      floor: 0.6,
      fetchImpl: aborting,
    });
    expect(timedOut.ok).toBe(false);
    if (!timedOut.ok) expect(timedOut.reason).toBe("timeout");
  });
});
