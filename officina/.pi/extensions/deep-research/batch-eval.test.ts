import { describe, it, expect } from "vitest";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runSubCoder } from "../subagent/spawn.ts";
import { allocateBudget } from "./budget.ts";
import { runResearchPhases, REASONING_ALLOWED_TOOLS, type Question } from "./pipeline.ts";
import { writeInstructionBlock } from "./prompts.ts";
import { computeHeuristics } from "./metrics.ts";
import { judgePrompt, parseJudge } from "./judge.ts";
import { topicsForRound, type Topic } from "./topics.ts";

// Batch evaluation harness — runs the PRODUCTION pipeline (pipeline.ts, the same
// code orchestrate() uses) across the topic set, writes a full markdown report
// per topic via a dedicated write sub-coder, then scores each with mechanical
// heuristics + a local-model judge. Artifacts land under a gitignored results
// dir; results.json aggregates. Resumable: a topic with a non-empty report.md is
// skipped, so a killed run continues where it left off.
//
// Gated + configured by env:
//   DR_EVAL=1            enable (off in the normal suite)
//   DR_ROUND=1|2         1 → the 10 round-1 topics; 2 → all 15
//   DR_M=6               max-agents budget (default 6, the reference shape)
//   DR_MODEL=<id>        optional model override (default: launcher default)
//   DR_ONLY=<id[,id]>    optional: run only these topic ids (smoke a single one)
// Run:
//   DR_EVAL=1 DR_ROUND=1 DR_M=6 npx vitest run .pi/extensions/deep-research/batch-eval.test.ts

const EVAL = process.env.DR_EVAL === "1";
const ROUND = (Number(process.env.DR_ROUND) === 2 ? 2 : 1) as 1 | 2;
const M = Number(process.env.DR_M) > 0 ? Number(process.env.DR_M) : 6;
const MODEL = process.env.DR_MODEL || undefined;
const ONLY = (process.env.DR_ONLY || "").split(",").map((s) => s.trim()).filter(Boolean);
const PER_TOPIC_TIMEOUT_MS = 2_400_000; // 40 min ceiling per topic

const RUNS_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "benchmarks", "deep_research_runs");
const runRoot = join(RUNS_ROOT, `round${ROUND}_m${M}`);

function writeJsonAtomic(path: string, obj: unknown): void {
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, JSON.stringify(obj, null, 2));
  renameSync(tmp, path);
}

// A neutral stand-in for the user: the eval has no human, so answer clarifying
// questions generically (unbiased + comparable across topics/rounds).
async function cannedAnswers(questions: Question[]): Promise<string> {
  return questions
    .map((q) => `Q: ${q.q}\nA: No strong preference — keep the scope general, current (2025-2026), and broadly useful.`)
    .join("\n\n");
}

interface SummaryRow {
  id: string;
  status: "ok" | "empty" | "error";
  reportWords: number;
  structureScore: number;
  judgeOverall: number;
  failedSubagents: number;
  wave1: number;
  gap: number;
  totalMs: number;
  error?: string;
}

function updateResults(row: SummaryRow): void {
  const path = join(runRoot, "results.json");
  let rows: SummaryRow[] = [];
  if (existsSync(path)) {
    try {
      // The file is an object { round, M, …, rows: [...] }; older code read it as
      // a bare array and silently reset each time. Accept both shapes.
      const parsed = JSON.parse(readFileSync(path, "utf-8"));
      rows = Array.isArray(parsed) ? parsed : Array.isArray(parsed?.rows) ? parsed.rows : [];
    } catch {
      rows = [];
    }
  }
  rows = rows.filter((r) => r.id !== row.id);
  rows.push(row);
  rows.sort((a, b) => a.id.localeCompare(b.id));
  writeJsonAtomic(path, { round: ROUND, M, model: MODEL || "(default)", generatedTopics: rows.length, rows });
}

async function runOneTopic(topic: Topic): Promise<void> {
  const dir = join(runRoot, topic.id);
  mkdirSync(dir, { recursive: true });
  const reportPath = join(dir, "report.md");

  // Resume: skip if a non-empty report already exists.
  if (existsSync(reportPath) && readFileSync(reportPath, "utf-8").trim().length > 0) {
    console.log(`[eval] ${topic.id}: already done — skipping`);
    return;
  }

  // Research/reasoning children run in an isolated scratch subdir (research
  // agents are bash-free now, but keep the topic dir clean of any incidental files).
  const scratch = join(dir, "scratch");
  mkdirSync(scratch, { recursive: true });
  const budget = allocateBudget(M);
  const t0 = Date.now();
  let row: SummaryRow = {
    id: topic.id,
    status: "error",
    reportWords: 0,
    structureScore: 0,
    judgeOverall: 0,
    failedSubagents: 0,
    wave1: 0,
    gap: 0,
    totalMs: 0,
  };

  const briefPath = join(dir, "brief.txt");
  const digestPath = join(dir, "digest.txt");
  const pmetaPath = join(dir, "pipeline-meta.json");

  try {
    // Phases 1-4 (production pipeline). Research is the long part and gets
    // CHECKPOINTED to disk (brief/digest/pipeline-meta) the moment it finishes,
    // so a job killed during the short write phase resumes here without re-running
    // ~18 min of research (this environment kills background jobs at ~15-20 min).
    let brief: string;
    let digest: string;
    let meta;
    if (existsSync(pmetaPath) && existsSync(digestPath) && existsSync(briefPath)) {
      brief = readFileSync(briefPath, "utf-8");
      digest = readFileSync(digestPath, "utf-8");
      meta = JSON.parse(readFileSync(pmetaPath, "utf-8"));
      console.log(`[eval] ${topic.id}: resuming from research checkpoint → write`);
    } else {
      const res = await runResearchPhases(topic.topic, {
        budget,
        cwd: scratch,
        model: MODEL,
        signal: new AbortController().signal,
        hooks: {
          askAnswers: cannedAnswers,
          onResearchProgress: (done, total) => console.log(`[eval] ${topic.id}: ${done}/${total} agents`),
        },
      });
      brief = res.brief;
      digest = res.digest;
      meta = res.meta;
      writeFileSync(briefPath, brief);
      writeFileSync(digestPath, digest);
      writeJsonAtomic(pmetaPath, meta);
    }

    // 5. Write — a dedicated write sub-coder; its r.report is the FULL report
    // (the 2000-char cap only applies to findings re-entering a parent context).
    const writeStart = Date.now();
    const writeRes = await runSubCoder({
      id: "write",
      label: "writer",
      task: writeInstructionBlock(topic.topic, brief, digest, meta.failedSubagents ?? 0),
      cwd: scratch,
      model: MODEL,
      reportSuffix: "",
      allowedTools: REASONING_ALLOWED_TOOLS, // write from findings; no web/bash
    });
    const report = (writeRes.report || "").trim();
    const writeMs = Date.now() - writeStart;
    writeFileSync(reportPath, report ? report + "\n" : "");

    const heur = computeHeuristics(report, meta);
    writeJsonAtomic(join(dir, "heuristics.json"), heur);

    // Judge (local model; lenient — directional signal).
    let judge = null;
    let judgeMs = 0;
    if (report) {
      const js = Date.now();
      const judgeRes = await runSubCoder({
        id: "judge",
        label: "judge",
        task: judgePrompt(brief, report),
        cwd: scratch,
        model: MODEL,
        reportSuffix: "",
        allowedTools: REASONING_ALLOWED_TOOLS,
      });
      judgeMs = Date.now() - js;
      judge = parseJudge(judgeRes.report);
      writeJsonAtomic(join(dir, "judge.json"), judge ?? { error: "unparseable", raw: judgeRes.report.slice(0, 500) });
    }

    writeJsonAtomic(join(dir, "meta.json"), {
      topic: topic.topic,
      budget,
      pipeline: meta,
      writeMs,
      judgeMs,
      writeExit: writeRes.exitCode,
      totalMs: Date.now() - t0,
    });

    row = {
      id: topic.id,
      status: report ? "ok" : "empty",
      reportWords: heur.words,
      structureScore: heur.structureScore,
      judgeOverall: judge?.overall ?? 0,
      failedSubagents: meta.failedSubagents,
      wave1: meta.wave1Count,
      gap: meta.gapCount,
      totalMs: Date.now() - t0,
    };
    console.log(
      `[eval] ${topic.id}: ${row.status} words=${row.reportWords} struct=${row.structureScore} ` +
        `judge=${row.judgeOverall} failed=${row.failedSubagents} ${Math.round(row.totalMs / 1000)}s`,
    );
    updateResults(row);
    // Empty report = a real failure worth surfacing (batch still continues).
    expect(report.length).toBeGreaterThan(0);
  } catch (e) {
    row.error = (e as Error)?.message ?? String(e);
    row.totalMs = Date.now() - t0;
    try {
      writeFileSync(join(dir, "error.txt"), row.error);
    } catch {
      /* ignore */
    }
    updateResults(row);
    throw e;
  }
}

describe.skipIf(!EVAL)(`deep-research batch eval — round ${ROUND}, M=${M}`, () => {
  mkdirSync(runRoot, { recursive: true });
  let topics = topicsForRound(ROUND);
  if (ONLY.length) topics = topics.filter((t) => ONLY.includes(t.id));
  for (const topic of topics) {
    it(topic.id, () => runOneTopic(topic), PER_TOPIC_TIMEOUT_MS);
  }
});
