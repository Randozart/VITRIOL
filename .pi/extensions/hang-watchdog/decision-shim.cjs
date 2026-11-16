#!/usr/bin/env node
// PROVENANCE: pattern from llama.cpp HTTP server.
// SystemOne-style watchdog decision shim — wraps a VITRIOL /v1/chat/completions
// endpoint into the structured decision contract expected by
// hang-watchdog's decision.ts.
//
// Strategy: use heuristic rules first (zero latency), call model only for
// ambiguous cases. This avoids saturating the daily-driver VITRIOL with
// classification traffic.
//
// Usage: node decision-shim.cjs [port] [vritiol-url]
//   port:       HTTP listen port (default 8381)
//   vritiol-url: VITRIOL base URL (default http://127.0.0.1:8382)
// Env: FLOOR=0.85, VITRIOL_TIMEOUT=ms

"use strict";
const http = require("node:http");

const JUDGE_PORT = parseInt(process.argv[2] || "8381", 10);
const VITRIOL_URL = process.argv[3] || "http://127.0.0.1:8382";
const FLOOR = parseFloat(process.env.FLOOR || "0.85");
const TIMEOUT_MS = parseInt(process.env.VITRIOL_TIMEOUT || "45000", 10);

const SYSTEM_PROMPT =
    "You are a watchdog judge. Reply with exactly one word: ALIVE, HUNG, or UNKNOWN. Nothing else.\n" +
    "ALIVE = command is clearly progressing (compiling, downloading, iterating)\n" +
    "HUNG = command is blocked (deadlocked, waiting for stdin, infinite repetition)\n" +
    "UNKNOWN = insufficient evidence, ambiguous, or stale.";

// ── Heuristics ────────────────────────────────────────────────────────────

/** Keyword-based heuristic classification. Returns null if ambiguous. */
function heuristic(classify) {
    const text = classify.text;
    // Clear HUNG signals
    if (
        /\b(stall|blocked|waiting|stdin|deadlock|frozen|no.+change|no.+output|no progress|hung)\b/.test(
            text,
        )
    ) {
        const stallRatio = classify.stalled_s / Math.max(1, classify.elapsed_s);
        if (stallRatio > 0.7) return { choice: "HUNG", confidence: 0.9 };
    }
    // Clear ALIVE signals
    if (
        /\b(compil|build|downl|progress|iterat|running|working|transf|fetch|fetching|loading|writing|sending|processing)\b/.test(
            text,
        )
    ) {
        const stallRatio = classify.stalled_s / Math.max(1, classify.elapsed_s);
        if (stallRatio < 0.3) return { choice: "ALIVE", confidence: 0.85 };
    }
    return null;
}

// ── Helpers ───────────────────────────────────────────────────────────────

function finite(v) {
    return Number.isFinite(v) ? v : NaN;
}

function clamp(v) {
    return isNaN(v) ? 0.5 : Math.max(0, Math.min(1, v));
}

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
                low_confidence: clamp(finite(confidence)) < FLOOR,
            },
        },
    };
}

// ── Server ────────────────────────────────────────────────────────────────

async function classifyViaModel(cmd, elapsed, stalled, tail) {
    const userMsg = `Command: ${cmd}\nElapsed: ${elapsed}s | Stalled: ${stalled}s\nOutput tail: ${tail}`;

    try {
        const resp = await fetch(`${VITRIOL_URL}/v1/chat/completions`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({
                model: "default",
                messages: [
                    { role: "system", content: SYSTEM_PROMPT },
                    { role: "user", content: userMsg },
                ],
                max_tokens: 8,
                temperature: 0.0,
                cache_prompt: false,
            }),
            signal: AbortSignal.timeout(TIMEOUT_MS),
        });

        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);

        const doc = await resp.json();
        const content = doc.choices?.[0]?.message?.content ?? "";
        return parseChoice(content);
    } catch {
        return null; // Model unavailable or timed out
    }
}

async function classifyEvidence(cmd, elapsed, stalled, tail) {
    const classify = {
        cmd: cmd.toLowerCase(),
        tail: tail.toLowerCase(),
        elapsed_s: elapsed,
        stalled_s: stalled,
        text: `${cmd} ${tail}`.toLowerCase(),
    };

    // Try heuristics first (zero latency, no GPU pressure)
    const h = heuristic(classify);
    if (h) return h;

    // Fall back to model for ambiguous cases
    return classifyViaModel(cmd, elapsed, stalled, tail);
}

const server = http.createServer(async (req, res) => {
    if (req.method !== "POST" || req.url !== "/systemone") {
        res.writeHead(404);
        res.end('{"error":"not found"}');
        return;
    }

    let body = "";
    for await (const chunk of req) body += chunk;
    let decReq;
    try {
        decReq = JSON.parse(body);
    } catch {
        res.writeHead(400);
        res.end('{"error":"invalid json"}');
        return;
    }

    const state = decReq.state || {};
    const cmd = (state.command || "").slice(0, 300);
    const tail = (state.output_tail || "").slice(0, 200);
    const elapsed = state.elapsed_s || 0;
    const stalled = state.stalled_s || 0;

    try {
        const result = await classifyEvidence(cmd, elapsed, stalled, tail);
        if (result) {
            const response = buildResponse(result.choice, result.confidence);
            res.writeHead(200, { "content-type": "application/json" });
            res.end(JSON.stringify(response));
        } else {
            // Model timed out — return UNKNOWN
            const fallback = buildResponse("UNKNOWN", 0);
            res.writeHead(502);
            res.end(JSON.stringify(fallback));
        }
    } catch {
        const fallback = buildResponse("UNKNOWN", 0);
        res.writeHead(502);
        res.end(JSON.stringify(fallback));
    }
});

server.listen(JUDGE_PORT, "127.0.0.1", () => {
    console.log(
        `Watchdog decision shim on :${JUDGE_PORT} → ${VITRIOL_URL}/v1/chat/completions (floor=${FLOOR}, timeout=${TIMEOUT_MS}ms)`,
    );
});
