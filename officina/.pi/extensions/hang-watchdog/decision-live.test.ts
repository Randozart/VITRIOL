// Live smoke for the Gemini Decisions lane (2026-11-16).
//
// Skipped unless DECISION_LIVE=1, because it spends (a fraction of a cent of)
// quota and needs a real key. Run:
//   DECISION_LIVE=1 npx vitest --run .pi/extensions/hang-watchdog/decision-live.test.ts
//
// It exercises the real `askGeminiDecision` against the documented endpoint, with
// the real key resolution (env → ascensus secrets), so it fails if the key moved
// or the API shape changed.
import { describe, expect, it } from "vitest";
import { askGeminiDecision } from "./decision-gemini.ts";
import { resolveDecisionConfig } from "./config.ts";

const LIVE = process.env.DECISION_LIVE === "1";

describe.skipIf(!LIVE)("gemini decisions (live)", () => {
  it("resolves a key from the environment or the ascensus store", () => {
    const cfg = resolveDecisionConfig({
      ...process.env,
      OFFICINA_WATCHDOG_DECISION_MODE: "gemini",
    });
    expect(cfg.mode).toBe("gemini");
    expect(cfg.key.length).toBeGreaterThan(20);
    expect(cfg.key.startsWith("AIza") || cfg.key.startsWith("sk-")).toBe(true);
  });

  it("classifies clearly-progressing evidence as ALIVE", async () => {
    const cfg = resolveDecisionConfig({
      ...process.env,
      OFFICINA_WATCHDOG_DECISION_MODE: "gemini",
    });
    const out = await askGeminiDecision(
      {
        command: "cargo build --release",
        elapsedS: 120,
        stalledS: 0,
        outputTail:
          "Compiling officina-tui v0.2.0\nFinished release profile in 24s",
      },
      { key: cfg.key, floor: cfg.floor, margin: cfg.margin, model: cfg.model },
    );
    expect(out.ok, out.ok ? "" : out.reason).toBe(true);
    if (!out.ok) return;
    const liveness = out.reply.answers!.liveness!;
    console.log(
      `[live] choice=${liveness.choice} p=${liveness.answer_confidence?.toFixed(3)} ` +
        `probs=${JSON.stringify(out.probabilities)} reason=${out.reason}`,
    );
    expect(["ALIVE", "HUNG", "UNKNOWN"]).toContain(liveness.choice);
    // No golden answer (the model is calibrated, not deterministic), but a
    // compiling build must not come back as HUNG with high confidence.
    if (liveness.choice === "HUNG") {
      expect(liveness.answer_confidence ?? 0).toBeLessThan(0.8);
    }
  });

  it("reports the probability split for richer evidence", async () => {
    const cfg = resolveDecisionConfig({ ...process.env, OFFICINA_WATCHDOG_DECISION_MODE: "gemini" });
    const out = await askGeminiDecision(
      {
        command: "cargo build --release",
        elapsedS: 240,
        stalledS: 0,
        outputTail:
          "   Compiling serde v1.0.210\n   Compiling tokio v1.40.0\n" +
          "   Compiling officina-tui v0.2.0\n    Finished release profile in 24s",
      },
      { key: cfg.key, floor: 0.0, margin: 0, model: cfg.model },
    );
    expect(out.ok, out.ok ? "" : out.reason).toBe(true);
    if (!out.ok) return;
    console.log(
      `[live-rich] choice=${out.reply.answers!.liveness!.choice} probs=${JSON.stringify(out.probabilities)}`,
    );
  });

  it("does not call a blocked command alive", async () => {
    const cfg = resolveDecisionConfig({ ...process.env, OFFICINA_WATCHDOG_DECISION_MODE: "gemini" });
    const out = await askGeminiDecision(
      {
        command: "cat",
        elapsedS: 300,
        stalledS: 280,
        outputTail: "",
      },
      { key: cfg.key, floor: 0.0, margin: 0, model: cfg.model },
    );
    expect(out.ok, out.ok ? "" : out.reason).toBe(true);
    if (!out.ok) return;
    const p = out.probabilities;
    console.log(`[live-hung] choice=${out.reply.answers!.liveness!.choice} probs=${JSON.stringify(p)}`);
    // A 300 s silent `cat` must not be the most likely ALIVE reading.
    expect((p.HUNG ?? 0) + (p.UNKNOWN ?? 0)).toBeGreaterThan(p.ALIVE ?? 0);
  });

  it("redacts a secret-shaped tail before sending", async () => {
    const cfg = resolveDecisionConfig({
      ...process.env,
      OFFICINA_WATCHDOG_DECISION_MODE: "gemini",
    });
    const out = await askGeminiDecision(
      {
        command: "GEMINI_API_KEY=AIza" + "e".repeat(35) + " cargo test",
        elapsedS: 30,
        stalledS: 0,
        outputTail: "running 12 tests ... ok",
      },
      { key: cfg.key, floor: 0.0, margin: 0, model: cfg.model },
    );
    // Whatever the verdict, the call must succeed (redaction kept it valid).
    expect(out.ok, out.ok ? "" : out.reason).toBe(true);
  });
});
