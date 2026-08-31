import { describe, it, expect } from "vitest";
import { runSubCoder, runSubCodersConcurrent, type SubCoderItem } from "../subagent/spawn.ts";
import { allocateBudget } from "./budget.ts";
import { extractJsonArray, digestFindings } from "./helpers.ts";
import { clarifyPrompt, briefPrompt, decomposePrompt, gapPrompt } from "./prompts.ts";

// Live engine test — spawns REAL child little-coders against the running local
// server (route_proxy :8888) and exercises the whole research pipeline end to
// end: clarify → brief → decompose → wave-1 research → gap analysis → digest.
//
// It does NOT cover the interactive TUI (f2, dialogs, progress bar) or the
// main-agent write turn — those need a real terminal + session and are validated
// manually. This de-risks the expensive part: that the reason-steps emit
// parseable JSON on this model and that research sub-coders return findings.
//
// Gated behind DR_LIVE=1 so the normal suite stays offline/fast. Run with:
//   DR_LIVE=1 npx vitest run .pi/extensions/deep-research/live-research.test.ts

const LIVE = process.env.DR_LIVE === "1";
const FAST = process.env.DR_FAST === "1";
const cwd = process.cwd();

// A deliberately small, knowledge-answerable topic so the smoke stays fast and
// doesn't hang on flaky browsing — mechanics, not depth (M=3 → wave1 1, reserve 1).
const TOPIC = "Briefly compare the Rust and Go programming languages for building CLI tools.";

// Fast reasoning-only check (no web-research child) — verifies the brief step
// emits a real scoping BRIEF, not findings/citations (the REPORT_SUFFIX-derail
// bug the first live run exposed). Run with:
//   DR_FAST=1 npx vitest run .pi/extensions/deep-research/live-research.test.ts
describe.skipIf(!FAST)("deep-research reasoning steps (fast, no web child)", () => {
  it(
    "brief step produces a scoping brief, not a report with fabricated citations",
    async () => {
      const brief = (
        await runSubCoder({
          id: "b",
          label: "brief",
          task: briefPrompt(TOPIC, "(no answers)"),
          cwd,
          reportSuffix: "",
        })
      ).report.trim();
      console.log(`[fast] brief (${brief.length} chars):\n${brief}\n---`);
      expect(brief.length).toBeGreaterThan(40);
      // A brief steers research; it must NOT itself carry findings/source URLs.
      expect(brief).not.toMatch(/https?:\/\//);
      expect(brief.toLowerCase()).not.toContain("## research report");
    },
    300_000,
  );
});

describe.skipIf(!LIVE)("deep-research live engine", () => {
  it(
    "runs clarify → brief → decompose → research → gap → digest against the live server",
    async () => {
      const budget = allocateBudget(3);
      expect(budget.wave1).toBe(1);
      expect(budget.reserve).toBe(1);

      // 1a. Clarify — must return a (possibly empty) JSON array, not crash.
      const clarify = await runSubCoder({ id: "c", label: "clarify", task: clarifyPrompt(TOPIC), cwd });
      const questions = extractJsonArray(clarify.report);
      console.log(`[live] clarify: ${questions.length} question(s), exit=${clarify.exitCode}`);
      expect(Array.isArray(questions)).toBe(true);

      // 1b. Brief — non-empty prose north star.
      const brief = (
        await runSubCoder({ id: "b", label: "brief", task: briefPrompt(TOPIC, "(no answers)"), cwd })
      ).report.trim();
      console.log(`[live] brief (${brief.length} chars):\n${brief.slice(0, 400)}\n---`);
      expect(brief.length).toBeGreaterThan(40);

      // 2. Decompose — the lead splits into wave1 subtopic tasks.
      const dec = await runSubCoder({ id: "d", label: "lead", task: decomposePrompt(brief, budget.wave1), cwd });
      const tasks = extractJsonArray(dec.report).filter((t) => t && typeof t.task === "string");
      console.log(`[live] decompose: ${tasks.length} task(s)`);
      expect(tasks.length).toBeGreaterThanOrEqual(1);

      // 3. Wave 1 — real research sub-coder(s).
      const items: SubCoderItem[] = tasks.slice(0, budget.wave1).map((t: any, i: number) => ({
        id: `w1-${i + 1}`,
        label: String(t.label || `area ${i + 1}`).slice(0, 24),
        task: String(t.task),
        cwd,
      }));
      const wave1 = await runSubCodersConcurrent(items, {});
      const ok = wave1.filter((r) => r.exitCode === 0 && r.report.trim().length > 0);
      console.log(`[live] wave1: ${ok.length}/${wave1.length} produced findings`);
      for (const r of wave1) console.log(`  - ${r.label} exit=${r.exitCode}: ${r.report.slice(0, 120)}`);
      expect(ok.length).toBeGreaterThanOrEqual(1);

      // 4. Gap analysis — must return a bounded JSON array (0..reserve), not crash.
      const gap = await runSubCoder({
        id: "g",
        label: "gap",
        task: gapPrompt(brief, digestFindings(wave1), budget.reserve),
        cwd,
      });
      const gapTasks = extractJsonArray(gap.report);
      console.log(`[live] gap: ${gapTasks.length} follow-up(s) proposed (≤${budget.reserve})`);
      expect(Array.isArray(gapTasks)).toBe(true);

      // 5. Digest assembles labeled findings for the writer.
      const digest = digestFindings(wave1);
      expect(digest).toContain("Finding 1:");
      console.log(`[live] digest (${digest.length} chars) assembled OK`);
    },
    900_000,
  );
});
