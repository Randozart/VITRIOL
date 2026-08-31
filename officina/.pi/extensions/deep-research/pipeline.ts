// UI-agnostic research phase engine — the single source of truth for the
// Scope → Research pipeline (phases 1-4). Both the interactive flow
// (index.ts orchestrate()) and the headless batch eval drive THIS function, so
// the eval measures the same code production runs. The WRITE phase (step 5) is
// intentionally NOT here: the interactive flow hands it to the main agent, and
// the eval writes via a dedicated sub-coder — both take {brief, digest} from the
// result below.
//
// Every reasoning step is a child little-coder (an extension can't call
// inference directly); research waves fan out read-only research sub-coders.
// UI is injected via hooks so this module has no ctx/widget dependency.

import {
  runSubCoder,
  runSubCodersConcurrent,
  SUBCODER_ALLOWED_TOOLS,
  type SubCoderItem,
  type SubCoderResult,
} from "../subagent/spawn.ts";
import type { Budget } from "./budget.ts";
import { extractJsonArray, digestFindings } from "./helpers.ts";
import {
  clarifyPrompt,
  briefPrompt,
  decomposePrompt,
  gapPrompt,
  singleAgentTask,
} from "./prompts.ts";

// Research children get the full read+browse toolset MINUS bash: web research
// needs webfetch/websearch/browser, never `bash` — with bash they scaffold and
// compile projects (cargo/go build) in the working tree, polluting it with
// hundreds of MB and stalling the run. Reasoning children (clarify/brief/
// decompose/gap/write) plan from given text only, so they get read-only LOCAL
// tools — no web, no bash — which keeps them fast, deterministic, side-effect-free.
export const RESEARCH_ALLOWED_TOOLS = SUBCODER_ALLOWED_TOOLS.split(",")
  .filter((t) => t !== "bash")
  .join(",");
export const REASONING_ALLOWED_TOOLS = "read,grep,glob,find,ls";

// Per-child watchdog timeouts. A hung research agent (e.g. a browser stuck on a
// page) would otherwise block the whole run forever — the eval hit exactly this.
// Reasoning steps are pure text and should be quick; research browses, so it gets
// a longer leash. On timeout the child is killed and marked failed; siblings and
// the rest of the pipeline continue.
export const REASONING_TIMEOUT_MS = 240_000; // 4 min
export const RESEARCH_TIMEOUT_MS = 600_000; // 10 min

// Report instructions for RESEARCH children — replaces the default coding-oriented
// REPORT_SUFFIX ("file:line citations"). Round 1 showed the fix this targets:
// research findings came back with ZERO source URLs, so the writer fabricated a
// Sources section and confabulated specifics (2026 model names + benchmark
// numbers). This demands real URLs and forbids inventing anything.
export const RESEARCH_SUFFIX =
  "\n\nReport back CONCISELY (≤ ~250 words) with your key findings for this task. Requirements:\n" +
  "- For every factual claim (numbers, names, versions, dates), cite the FULL source URL you read it from " +
  "(https://…), inline in parentheses — a bare domain is not enough.\n" +
  "- End with a 'Sources:' list of the actual URLs you visited (one per line).\n" +
  "- State ONLY what you actually found in sources you read. If you could not verify something, say so " +
  "explicitly rather than guessing. Do NOT fabricate names, numbers, versions, or URLs — an honest " +
  "'not found' is better than an invented fact.";

export interface Question {
  q: string;
  options: string[];
}

interface ResearchTask {
  label: string;
  task: string;
}

export interface PipelineHooks {
  /** Collect the user's answers to the clarifying questions (returns a joined string). */
  askAnswers: (questions: Question[]) => Promise<string>;
  /** A reasoning phase started (clarify/brief/decompose/gap) — drive a spinner. */
  onReasoning?: (message: string) => void;
  /** Research waves are about to run — initialize a progress bar to `total` agents. */
  onResearchBegin?: (total: number) => void;
  /** A research agent finished — update the bar. */
  onResearchProgress?: (done: number, total: number, phase: string) => void;
  /** Research is done — tear down the bar. */
  onResearchEnd?: () => void;
}

export interface PipelineOptions {
  budget: Budget;
  cwd: string;
  model: string | undefined;
  signal: AbortSignal;
  hooks: PipelineHooks;
}

export interface PipelineMeta {
  questionCount: number;
  wave1Count: number;
  gapCount: number;
  failedSubagents: number;
  /** Times a JSON reasoning step failed to parse and fell back to a default. */
  jsonFallbacks: number;
  phaseMs: Record<string, number>;
  totalMs: number;
}

export interface PipelineResult {
  brief: string;
  findings: SubCoderResult[];
  digest: string;
  meta: PipelineMeta;
  /** True if the run was aborted mid-flight (partial result). */
  aborted: boolean;
}

// One reasoning step = one read-only sub-coder returning its report text.
// reportSuffix:"" suppresses REPORT_SUFFIX so the "reply with a cited report"
// framing doesn't derail these steps into answering the topic.
async function reason(task: string, cwd: string, model: string | undefined, signal: AbortSignal): Promise<string> {
  const r = await runSubCoder({
    id: "r",
    label: "lead",
    task,
    cwd,
    model,
    signal,
    reportSuffix: "",
    allowedTools: REASONING_ALLOWED_TOOLS,
    timeoutMs: REASONING_TIMEOUT_MS,
  });
  return r.report;
}

function parseTasks(text: string, cap: number, fallbackLabel: string): ResearchTask[] {
  return extractJsonArray(text)
    .filter((t) => t && typeof t.task === "string")
    .slice(0, cap)
    .map((t, i) => ({
      label: String(t.label || `${fallbackLabel} ${i + 1}`).slice(0, 24),
      task: String(t.task),
    }));
}

export async function generateQuestions(
  topic: string,
  cwd: string,
  model: string | undefined,
  signal: AbortSignal,
): Promise<Question[]> {
  const text = await reason(clarifyPrompt(topic), cwd, model, signal);
  return extractJsonArray(text)
    .filter((q) => q && typeof q.q === "string")
    .slice(0, 4)
    .map((q) => ({
      q: String(q.q),
      options: (Array.isArray(q.options) ? q.options : []).map((o: any) => String(o)).filter(Boolean).slice(0, 3),
    }));
}

const countFailures = (rs: SubCoderResult[]) => rs.filter((r) => r.exitCode !== 0).length;

export async function runResearchPhases(topic: string, opts: PipelineOptions): Promise<PipelineResult> {
  const { budget, cwd, model, signal, hooks } = opts;
  const t0 = Date.now();
  const phaseMs: Record<string, number> = {};
  const time = async <T>(name: string, fn: () => Promise<T>): Promise<T> => {
    const s = Date.now();
    try {
      return await fn();
    } finally {
      phaseMs[name] = (phaseMs[name] ?? 0) + (Date.now() - s);
    }
  };
  const meta: PipelineMeta = {
    questionCount: 0,
    wave1Count: 0,
    gapCount: 0,
    failedSubagents: 0,
    jsonFallbacks: 0,
    phaseMs,
    totalMs: 0,
  };
  let currentBrief = topic;
  const done = (findings: SubCoderResult[], aborted: boolean): PipelineResult => {
    meta.failedSubagents = countFailures(findings);
    meta.totalMs = Date.now() - t0;
    return { brief: currentBrief, findings, digest: digestFindings(findings), meta, aborted };
  };

  // 1a. Scope — clarifying questions.
  hooks.onReasoning?.("scoping the research…");
  const questions = await time("clarify", () => generateQuestions(topic, cwd, model, signal));
  meta.questionCount = questions.length;
  if (signal.aborted) return done([], true);
  const answers = questions.length > 0 ? await hooks.askAnswers(questions) : "(no clarifying questions)";
  if (signal.aborted) return done([], true);

  // 1b. Scope — compress into the research brief.
  hooks.onReasoning?.("writing the research brief…");
  currentBrief = (await time("brief", () => reason(briefPrompt(topic, answers), cwd, model, signal))).trim() || topic;
  if (signal.aborted) return done([], true);

  let findings: SubCoderResult[];

  if (budget.single) {
    // M=1 — a single agent researches the whole brief.
    hooks.onResearchBegin?.(1);
    const items: SubCoderItem[] = [{ id: "1", label: "research", task: singleAgentTask(currentBrief), cwd }];
    findings = await time("wave1", () =>
      runSubCodersConcurrent(items, {
        model,
        signal,
        allowedTools: RESEARCH_ALLOWED_TOOLS,
        timeoutMs: RESEARCH_TIMEOUT_MS,
        retryOnTimeout: true,
        reportSuffix: RESEARCH_SUFFIX,
        onUpdate: (all) => hooks.onResearchProgress?.(all.filter((r) => r.exitCode !== -1).length, 1, "researching"),
      }),
    );
    meta.wave1Count = 1;
    hooks.onResearchEnd?.();
    return done(findings, signal.aborted);
  }

  // 2. Lead — decompose into wave-1 subtopics (the "1 agent starts").
  hooks.onReasoning?.("planning subtopics…");
  const targetsRaw = await time("decompose", () => reason(decomposePrompt(currentBrief, budget.wave1), cwd, model, signal));
  if (signal.aborted) return done([], true);
  let targets = parseTasks(targetsRaw, budget.wave1, "area");
  if (targets.length === 0) {
    meta.jsonFallbacks++;
    targets = [{ label: "research", task: singleAgentTask(currentBrief) }];
  }
  meta.wave1Count = targets.length;

  // Bar total = lead(1) + wave1 + reserve (updated down once wave 2 is sized).
  let total = 1 + targets.length + budget.reserve;
  hooks.onResearchBegin?.(total);
  hooks.onResearchProgress?.(1, total, "researching"); // lead done

  // 3. Research wave 1.
  const wave1Items: SubCoderItem[] = targets.map((t, i) => ({ id: `w1-${i + 1}`, label: t.label, task: t.task, cwd }));
  const wave1 = await time("wave1", () =>
    runSubCodersConcurrent(wave1Items, {
      model,
      signal,
      allowedTools: RESEARCH_ALLOWED_TOOLS,
      timeoutMs: RESEARCH_TIMEOUT_MS,
      retryOnTimeout: true,
      reportSuffix: RESEARCH_SUFFIX,
      onUpdate: (all) => hooks.onResearchProgress?.(1 + all.filter((r) => r.exitCode !== -1).length, total, "researching"),
    }),
  );
  if (signal.aborted) {
    hooks.onResearchEnd?.();
    return done(wave1, true);
  }

  // 4. Gap analysis + optional wave 2.
  let gap: SubCoderResult[] = [];
  if (budget.reserve > 0) {
    hooks.onReasoning?.("checking for gaps…");
    const gapRaw = await time("gap", () =>
      reason(gapPrompt(currentBrief, digestFindings(wave1), budget.reserve), cwd, model, signal),
    );
    if (signal.aborted) {
      hooks.onResearchEnd?.();
      return done(wave1, true);
    }
    const gapTasks = parseTasks(gapRaw, budget.reserve, "gap");
    meta.gapCount = gapTasks.length;
    total = 1 + wave1.length + gapTasks.length;
    const doneBase = 1 + wave1.length;
    hooks.onResearchProgress?.(doneBase, total, gapTasks.length > 0 ? "filling gaps…" : "wrapping up");
    if (gapTasks.length > 0) {
      const gapItems: SubCoderItem[] = gapTasks.map((t, i) => ({ id: `w2-${i + 1}`, label: t.label, task: t.task, cwd }));
      gap = await time("wave2", () =>
        runSubCodersConcurrent(gapItems, {
          model,
          signal,
          allowedTools: RESEARCH_ALLOWED_TOOLS,
          timeoutMs: RESEARCH_TIMEOUT_MS,
          retryOnTimeout: true,
          reportSuffix: RESEARCH_SUFFIX,
          onUpdate: (all) =>
            hooks.onResearchProgress?.(doneBase + all.filter((r) => r.exitCode !== -1).length, total, "filling gaps…"),
        }),
      );
    }
  }
  hooks.onResearchEnd?.();
  return done([...wave1, ...gap], signal.aborted);
}
