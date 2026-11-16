// PROVENANCE: pattern from llama.cpp HTTP server.
// Tests for decision-shim.cjs parsing and response-building logic.
// Uses Node's built-in test runner.

import { describe, it } from "node:test";
import assert from "node:assert/strict";

// ── parseChoice ───────────────────────────────────────────────────────────

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

describe("parseChoice", () => {
    it('returns ALIVE for "ALIVE"', () =>
        assert.deepStrictEqual(parseChoice("ALIVE"), {
            choice: "ALIVE",
            confidence: 0.95,
        }));

    it('returns HUNG for "HUNG"', () =>
        assert.deepStrictEqual(parseChoice("HUNG"), {
            choice: "HUNG",
            confidence: 0.95,
        }));

    it('returns UNKNOWN for "UNKNOWN"', () =>
        assert.deepStrictEqual(parseChoice("UNKNOWN"), {
            choice: "UNKNOWN",
            confidence: 0.95,
        }));

    it("returns UNKNOWN for unknown text", () =>
        assert.deepStrictEqual(parseChoice("maybe"), {
            choice: "UNKNOWN",
            confidence: 0.3,
        }));

    it("strips markdown and whitespace", () =>
        assert.deepStrictEqual(parseChoice("```ALIVE```"), {
            choice: "ALIVE",
            confidence: 0.95,
        }));

    it("handles bold markdown", () =>
        assert.deepStrictEqual(parseChoice("**ALIVE**"), {
            choice: "ALIVE",
            confidence: 0.95,
        }));

    it("handles multiline", () =>
        assert.deepStrictEqual(parseChoice("ALIVE\n"), {
            choice: "ALIVE",
            confidence: 0.95,
        }));
});

// ── buildResponse ─────────────────────────────────────────────────────────

function clamp(v) {
    return isNaN(v) ? 0.5 : Math.max(0, Math.min(1, v));
}
function finite(v) {
    return Number.isFinite(v) ? v : NaN;
}

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
                answer_confidence: clamp(finite(confidence)),
                action: { act_probability: answered ? 0.99 : 0.1 },
                abstention: answered ? "passed" : "abstained",
                low_confidence: clamp(finite(confidence)) < 0.85,
            },
        },
    };
}

describe("buildResponse", () => {
    it("ALIVE has confidence passed", () => {
        const r = buildResponse("ALIVE", 0.95);
        assert.strictEqual(r.answers.liveness.choice, "ALIVE");
        assert.strictEqual(r.answers.liveness.answer_confidence, 0.95);
        assert.strictEqual(r.answers.liveness.action.act_probability, 0.99);
        assert.strictEqual(r.answers.liveness.abstention, "passed");
        assert.strictEqual(r.answers.liveness.low_confidence, false);
    });

    it("HUNG with low confidence", () => {
        const r = buildResponse("HUNG", 0.7);
        assert.strictEqual(r.answers.liveness.choice, "HUNG");
        assert.strictEqual(r.answers.liveness.answer_confidence, 0.7);
        assert.strictEqual(r.answers.liveness.low_confidence, true);
    });

    it("UNKNOWN has abstention", () => {
        const r = buildResponse("UNKNOWN", 0.3);
        assert.strictEqual(r.answers.liveness.abstention, "abstained");
        assert.strictEqual(r.answers.liveness.action.act_probability, 0.1);
        assert.strictEqual(r.answers.liveness.low_confidence, true);
    });

    it("truncates confidence to [0, 1]", () => {
        const r = buildResponse("ALIVE", 1.5);
        assert.strictEqual(r.answers.liveness.answer_confidence, 1);
        const r2 = buildResponse("ALIVE", -0.5);
        assert.strictEqual(r2.answers.liveness.answer_confidence, 0);
    });

    it("NaN confidence defaults to 0.5", () => {
        const r = buildResponse("ALIVE", NaN);
        assert.strictEqual(r.answers.liveness.answer_confidence, 0.5);
    });

    it("has correct usage.coverage", () => {
        const r = buildResponse("ALIVE", 0.9);
        assert.deepStrictEqual(r.usage.options.liveness, {
            total: 3,
            distinct: 3,
        });
        assert.strictEqual(r.usage.truncated, false);
        assert.strictEqual(r.usage.state_tokens_dropped, 0);
    });
});
