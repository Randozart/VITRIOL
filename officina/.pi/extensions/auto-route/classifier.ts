// PROVENANCE: inspiration — intel/intel-ai-builder (Apache-2.0), Auto Route's
// per-task classification pattern: complexity + data-sensitivity signals scored
// before every model call, routing across execution tiers. Intel's router is
// closed-source (confirmed by Dr. Olena Zhu, The Neuron podcast 2026-08-05);
// only the documented architecture informed this design. Signal selection,
// scoring weights, and the privacy regexes are original VITRIOL work.
//
// classifier — pure turn classification for auto-route. No I/O, no pi runtime.
//
// Produces a complexity score (0..1) and a privacy class from signals cheaply
// available at the `context` hook: last user prompt text, recent tool-result
// errors, the edit-churn loop count, and the scratchpad's context section.
// Deliberately heuristic — the goal is a routing hint, not a benchmark.

export interface ClassifyInput {
  /** Text of the most recent user message ("" when none). */
  promptText: string;
  /** Number of error tool results among the most recent tool results. */
  recentErrorCount: number;
  /** Times the edit-churn detector fired this session. */
  churnLoops: number;
  /** Lines currently in the scratchpad `context` section (working-set data). */
  scratchpadContextLines: string[];
  /** Distinct files touched this session (session-ledger window). */
  filesTouched: number;
  /** Session turn count so far. */
  turnCount: number;
}

export type PrivacyClass = "safe" | "sensitive" | "confidential";

export interface Classification {
  complexity: number; // 0..1
  privacy: PrivacyClass;
  signals: Record<string, number>;
}

/** Long prompts correlate with multi-part requests. */
const PROMPT_MEDIUM = 500;
const PROMPT_HIGH = 2000;

/** Working-set pressure: many scratchpad context lines = a hairy task. */
const CONTEXT_LINES_MEDIUM = 8;
const CONTEXT_LINES_HIGH = 20;

/** Domain keywords that historically predict locally-unanswerable depth. */
const DEEP_DOMAIN_WORDS = [
  "cuda", "kernel", "driver", "systemd", "pcie", "vmm", "mlock",
  "segfault", "race condition", "deadlock", "ubsan", "valgrind",
  "linker", "abi", "opcode", "sm_61", "sm_86", "dmesg",
];

const SECRET_PATH_RE =
  /(^|[\s"'(=:])([\w./-]*)(\.env|\.ssh|\.aws|secrets?|credentials?|\.pem|\.p12|\.p8|\.key|keystore)([\w./-]*)/i;

const API_KEY_RE = /\b(AIzaSy[\w-]{20,}|sk-[\w-]{20,}|ghp_[\w]{20,}|github_pat_[\w]{20,}|xox[bp]-[\w-]{10,})\b/;

const PII_RE = /\b[\w.+-]+@[\w-]+\.[\w.]{2,}\b|\b\+?\d[\d\s().-]{7,}\d\b/;

const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));

/** Classify one turn. Pure — every field is computed from the input. */
export function classifyTurn(input: ClassifyInput): Classification {
  const signals: Record<string, number> = {};
  let complexity = 0.15; // baseline: a turn is never free

  const promptLen = input.promptText.length;
  signals.prompt_len = promptLen;
  if (promptLen > PROMPT_HIGH) {
    complexity += 0.2;
    signals.prompt_long = 1;
  } else if (promptLen > PROMPT_MEDIUM) {
    complexity += 0.1;
    signals.prompt_medium = 1;
  }

  const ctxLines = input.scratchpadContextLines.length;
  signals.scratchpad_context_lines = ctxLines;
  if (ctxLines > CONTEXT_LINES_HIGH) complexity += 0.2;
  else if (ctxLines > CONTEXT_LINES_MEDIUM) complexity += 0.1;

  signals.recent_errors = input.recentErrorCount;
  if (input.recentErrorCount >= 2) complexity += 0.25;
  else if (input.recentErrorCount === 1) complexity += 0.1;

  signals.churn_loops = input.churnLoops;
  if (input.churnLoops > 0) {
    complexity += 0.3; // the strongest "stuck locally" signal we have
    signals.loop_detected = 1;
  }

  signals.files_touched = input.filesTouched;
  if (input.filesTouched > 5) complexity += 0.05;

  signals.turn_count = input.turnCount;
  if (input.turnCount > 80) complexity += 0.05; // session fatigue

  const lower = input.promptText.toLowerCase();
  const domainHits = DEEP_DOMAIN_WORDS.filter((w) => lower.includes(w));
  signals.deep_domain_hits = domainHits.length;
  if (domainHits.length >= 3) complexity += 0.2;
  else if (domainHits.length > 0) complexity += 0.1;

  // ── privacy: most severe class wins; checked independently of complexity ──
  let privacy: PrivacyClass = "safe";
  if (SECRET_PATH_RE.test(input.promptText) || API_KEY_RE.test(input.promptText)) {
    privacy = "confidential";
    signals.privacy_confidential = 1;
  } else if (PII_RE.test(input.promptText)) {
    privacy = "sensitive";
    signals.privacy_sensitive = 1;
  }

  return { complexity: clamp01(complexity), privacy, signals };
}

/** Parse the `context` section out of scratchpad markdown (file `## x` or
 *  rendered `### x` headings). */
export function scratchpadContextLines(rendered: string): string[] {
  const lines: string[] = [];
  let inContext = false;
  for (const raw of rendered.split("\n")) {
    const line = raw.trim();
    const head = /^#{2,3} (.+)$/.exec(line);
    if (head) {
      inContext = head[1].trim() === "context";
      continue;
    }
    if (inContext && line.startsWith("- ")) lines.push(line.slice(2));
  }
  return lines;
}
