// Review findings contract (codex-qol stage 6, 2026-11-12). The model
// returns exactly one fenced ```officina-review JSON block; this module
// parses it strictly and persists it as .officina/review.json (versioned,
// atomic, 0600 — the Rust findings modal deserializes the same shape).

import { mkdirSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export type Severity = "high" | "medium" | "low" | "info";

export interface ReviewFinding {
  severity: Severity;
  file: string;
  line?: string;
  title: string;
  detail: string;
}

export interface ReviewFindings {
  version: 1;
  preset: string;
  created_at: string;
  summary: string;
  findings: ReviewFinding[];
}

const SEVERITIES = new Set(["high", "medium", "low", "info"]);

/** Extract + validate the ```officina-review block from model output.
 *  Strict: anything malformed is null (the caller reports honestly). */
export function parseFindingsBlock(text: string, preset = ""): ReviewFindings | null {
  const fence = /```officina-review\s*\n([\s\S]*?)```/.exec(text);
  if (!fence) return null;
  const raw = fence[1];
  const start = raw.indexOf("{");
  const end = raw.lastIndexOf("}");
  if (start < 0 || end <= start) return null;
  let doc: unknown;
  try {
    doc = JSON.parse(raw.slice(start, end + 1));
  } catch {
    return null;
  }
  return validateFindings(doc, preset);
}

export function validateFindings(doc: unknown, preset = ""): ReviewFindings | null {
  if (!doc || typeof doc !== "object") return null;
  const d = doc as Record<string, unknown>;
  if (d.version !== 1) return null;
  if (typeof d.summary !== "string") return null;
  if (!Array.isArray(d.findings)) return null;
  const findings: ReviewFinding[] = [];
  for (const f of d.findings) {
    if (!f || typeof f !== "object") return null;
    const row = f as Record<string, unknown>;
    const severity = String(row.severity ?? "");
    if (!SEVERITIES.has(severity)) return null;
    const file = String(row.file ?? "");
    const title = String(row.title ?? "");
    const detail = String(row.detail ?? "");
    if (!file || !title || !detail) return null;
    const finding: ReviewFinding = {
      severity: severity as Severity,
      file,
      title,
      detail,
    };
    if (row.line !== undefined && row.line !== null) {
      finding.line = String(row.line);
    }
    findings.push(finding);
  }
  return {
    version: 1,
    preset,
    created_at: new Date().toISOString(),
    summary: d.summary,
    findings,
  };
}

/** Atomic 0600 write of the findings document. */
export function writeFindings(dir: string, findings: ReviewFindings): string {
  mkdirSync(dir, { recursive: true });
  const path = join(dir, "review.json");
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, `${JSON.stringify(findings, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
  return path;
}

/** One-line summary for the notify: counts by severity. */
export function summarizeFindings(findings: ReviewFindings): string {
  const counts = { high: 0, medium: 0, low: 0, info: 0 };
  for (const f of findings.findings) counts[f.severity] += 1;
  const parts: string[] = [];
  if (counts.high) parts.push(`${counts.high} high`);
  if (counts.medium) parts.push(`${counts.medium} medium`);
  if (counts.low) parts.push(`${counts.low} low`);
  if (counts.info) parts.push(`${counts.info} info`);
  return `${findings.findings.length} finding(s)${parts.length ? ` · ${parts.join(", ")}` : ""}`;
}
