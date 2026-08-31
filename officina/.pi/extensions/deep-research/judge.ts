// Local-model LLM-as-judge — the second grade signal (the first is mechanical
// heuristics, the third is a human read). One caveat baked in: the judge is the
// SAME local model that wrote the report, so it is lenient and non-adversarial;
// treat its scores as directional, weighted below the human read. Run the prompt
// via runSubCoder(reportSuffix:"") from the harness; parse the reply here.

import { extractJsonObject } from "./helpers.ts";

export interface JudgeScores {
  coverage: number; // did it answer everything the brief asked?
  grounding: number; // claims backed by inline citations / sources?
  coherence: number; // single voice, well-structured, no disjointedness?
  usefulness: number; // actionable, appropriately deep?
  correctness: number; // plausibly accurate, no obvious contradictions/hallucination?
  notes: string;
  /** Mean of the five 1-5 dimensions. */
  overall: number;
}

export const JUDGE_DIMENSIONS = ["coverage", "grounding", "coherence", "usefulness", "correctness"] as const;

export function judgePrompt(brief: string, report: string): string {
  return (
    `Do not use any tools. You are grading a research report against the brief it was written for. ` +
    `Score each dimension from 1 (poor) to 5 (excellent), being critical and honest:\n` +
    `- coverage: are all of the brief's sub-questions actually answered?\n` +
    `- grounding: are the claims backed by inline [n] citations and a real Sources list?\n` +
    `- coherence: one neutral voice, logically structured, no repetition or disjointedness?\n` +
    `- usefulness: actionable and appropriately deep for the topic?\n` +
    `- correctness: plausibly accurate, no obvious contradictions or fabricated-looking facts?\n\n` +
    `Output ONLY a JSON object: ` +
    `{"coverage":n,"grounding":n,"coherence":n,"usefulness":n,"correctness":n,"notes":"<one sentence>"}. No prose.\n\n` +
    `### Brief\n${brief}\n\n### Report\n${report}`
  );
}

const clamp5 = (n: any): number => {
  const v = Number(n);
  if (!Number.isFinite(v)) return 0;
  return Math.max(1, Math.min(5, Math.round(v)));
};

export function parseJudge(text: string): JudgeScores | null {
  const o = extractJsonObject(text);
  if (!o) return null;
  const scores = {
    coverage: clamp5(o.coverage),
    grounding: clamp5(o.grounding),
    coherence: clamp5(o.coherence),
    usefulness: clamp5(o.usefulness),
    correctness: clamp5(o.correctness),
    notes: typeof o.notes === "string" ? o.notes.slice(0, 300) : "",
  };
  // Reject if the model didn't actually produce numeric dimensions.
  const dims = JUDGE_DIMENSIONS.map((d) => (scores as any)[d]);
  if (dims.every((v) => v === 0)) return null;
  const rated = dims.filter((v) => v > 0);
  const overall = rated.length ? Math.round((rated.reduce((a, b) => a + b, 0) / rated.length) * 10) / 10 : 0;
  return { ...scores, overall };
}
