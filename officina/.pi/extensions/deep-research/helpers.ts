// Pure helpers for the deep-research flow — no side effects, unit-tested.

import { truncateReport, type SubCoderResult } from "../subagent/spawn.ts";
import { clampAgents, MAX_AGENTS } from "./budget.ts";

// Pull the first balanced JSON array out of a model reply (small local models
// love to wrap JSON in prose / code fences). Returns [] on failure so callers
// can fall back to a default. Same contract as plan-mode's extractJsonArray.
export function extractJsonArray(text: string): any[] {
  const start = text.indexOf("[");
  const end = text.lastIndexOf("]");
  if (start < 0 || end <= start) return [];
  try {
    const v = JSON.parse(text.slice(start, end + 1));
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

// Pull the first balanced JSON OBJECT out of a model reply (sibling to
// extractJsonArray, for the rubric judge which returns an object). Returns null
// on failure so callers can fall back.
export function extractJsonObject(text: string): any | null {
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end <= start) return null;
  try {
    const v = JSON.parse(text.slice(start, end + 1));
    return v && typeof v === "object" && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

// A filesystem-safe slug for the report filename, derived from the topic.
export function slugify(topic: string): string {
  const s = (topic || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+/, "")
    .slice(0, 48)
    .replace(/-+$/, "");
  return s || "research";
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

// deep-research-<slug>-<YYYYMMDD-HHMMSS>.md — a timestamp keeps repeated runs on
// the same topic from clobbering each other. `now` is injected for testability.
export function reportFilename(topic: string, now: Date): string {
  const ts =
    `${now.getFullYear()}${pad2(now.getMonth() + 1)}${pad2(now.getDate())}` +
    `-${pad2(now.getHours())}${pad2(now.getMinutes())}${pad2(now.getSeconds())}`;
  return `deep-research-${slugify(topic)}-${ts}.md`;
}

// Compress all sub-coder findings into a single labeled digest for the writer.
// Failed children are marked (not dropped) so the writer knows what's missing —
// same "don't silently lose a subagent" rule as plan-mode's digestReports.
//
// Findings are truncated at 3500 chars (not the default 2000): each now ends with
// a Sources URL list, and the tighter cap was clipping those URLs before they
// reached the writer — defeating the grounding fix. The digest feeds the writer,
// not a context-constrained parent, so the extra room is cheap.
const DIGEST_FINDING_CHARS = 3500;
export function digestFindings(results: SubCoderResult[]): string {
  return results
    .map((r, i) => {
      const head = `### Finding ${i + 1}: ${r.label}`;
      const body =
        r.exitCode === 0
          ? truncateReport(r.report, DIGEST_FINDING_CHARS) || "(no findings returned)"
          : `(research failed: ${r.errorMessage || "no output"})`;
      return `${head}\n${body}`;
    })
    .join("\n\n");
}

// Resolve the default max-agents cap: env override wins, then settings, then 10.
// Pure — env and settings are passed in so the index wiring stays thin.
export function resolveMaxDefault(
  env: Record<string, string | undefined>,
  settings: { deep_research?: { default_max_subagents?: number } } | null | undefined,
): number {
  const raw = env.LITTLE_CODER_DEEP_RESEARCH_MAX;
  if (raw !== undefined && raw !== "") {
    const n = Number(raw);
    if (Number.isFinite(n)) return clampAgents(n);
  }
  const s = settings?.deep_research?.default_max_subagents;
  if (typeof s === "number" && Number.isFinite(s)) return clampAgents(s);
  return MAX_AGENTS;
}
