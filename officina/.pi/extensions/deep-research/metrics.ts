// Pure mechanical heuristics over a finished markdown report — the objective
// leg of the three-signal grade (heuristics + local judge + human read). No I/O,
// unit-tested.

import type { PipelineMeta } from "./pipeline.ts";

const URL_RE = /https?:\/\/[^\s)\]]+/g;
const INLINE_CITE_RE = /\[(\d{1,3})\]/g;
// Self-referential "the agents found / I found / as an AI" phrasing the report
// rules forbid (the report must read as one neutral voice).
const SELF_REF_RE = /\b(i found|i researched|the agents?|as an ai|the sub-?coders?|my research)\b/i;

export function wordCount(report: string): number {
  const t = report.trim();
  return t ? t.split(/\s+/).length : 0;
}

/** Total inline [n] citation markers (not unique — density of grounding). */
export function citationCount(report: string): number {
  return (report.match(INLINE_CITE_RE) || []).length;
}

/** Distinct citation numbers actually referenced inline. */
export function distinctCitations(report: string): number {
  const nums = new Set<string>();
  let m: RegExpExecArray | null;
  const re = new RegExp(INLINE_CITE_RE);
  while ((m = re.exec(report)) !== null) nums.add(m[1]);
  return nums.size;
}

/** Unique source URLs anywhere in the report (the Sources list, mostly). */
export function uniqueSources(report: string): number {
  const urls = report.match(URL_RE) || [];
  return new Set(urls.map((u) => u.replace(/[.,;]+$/, ""))).size;
}

export function hasSourcesSection(report: string): boolean {
  return /(^|\n)#{2,4}\s*sources\b/i.test(report);
}

export function hasH1(report: string): boolean {
  return /(^|\n)#\s+\S/.test(report);
}

/** A markdown table = a header row of pipes followed by a `---|---` separator. */
export function hasTable(report: string): boolean {
  return /(^|\n)\s*\|.*\|.*\n\s*\|?[\s:]*-{2,}[-|\s:]*\|/.test(report);
}

export function hasSelfReference(report: string): boolean {
  return SELF_REF_RE.test(report);
}

export interface ReportHeuristics {
  words: number;
  citations: number;
  distinctCitations: number;
  uniqueSources: number;
  hasSourcesSection: boolean;
  hasH1: boolean;
  hasTable: boolean;
  hasSelfReference: boolean;
  failedSubagents: number;
  jsonFallbacks: number;
  /** A crude 0-100 structural-quality score for quick sorting (NOT the grade). */
  structureScore: number;
}

export function computeHeuristics(report: string, meta: PipelineMeta): ReportHeuristics {
  const words = wordCount(report);
  const distinct = distinctCitations(report);
  const sources = uniqueSources(report);
  const h: Omit<ReportHeuristics, "structureScore"> = {
    words,
    citations: citationCount(report),
    distinctCitations: distinct,
    uniqueSources: sources,
    hasSourcesSection: hasSourcesSection(report),
    hasH1: hasH1(report),
    hasTable: hasTable(report),
    hasSelfReference: hasSelfReference(report),
    failedSubagents: meta.failedSubagents,
    jsonFallbacks: meta.jsonFallbacks,
  };
  // Structure score: rewards the report rules (title, sources, citations, length,
  // grounding) and penalizes self-reference / failed research. Bounded 0-100.
  let s = 0;
  if (h.hasH1) s += 10;
  if (h.hasSourcesSection) s += 20;
  if (h.hasTable) s += 10;
  s += Math.min(20, distinct * 3); // grounding breadth
  s += Math.min(15, sources * 2); // real sources
  if (words >= 500) s += 15;
  if (words >= 1200) s += 10;
  if (h.hasSelfReference) s -= 15;
  s -= Math.min(20, h.failedSubagents * 10);
  h.jsonFallbacks && (s -= 5);
  return { ...h, structureScore: Math.max(0, Math.min(100, s)) };
}
