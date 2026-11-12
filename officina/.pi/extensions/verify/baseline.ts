// verify — baseline diff + classification + report rendering (pure).
//
// THE WIP CONTRACT (plan invariant I1): failures are classified, never
// binary. A gate's FIRST observation in the session is its baseline
// (session-start cheap run captures the pre-edit reality for auto gates);
// everything later diffs against it:
//   - key in baseline            → pre-existing  (count, never alarming)
//   - new + file touched by you  → in-progress   (amber — expected mid-work)
//   - new + file untouched       → regression    (red — you broke bystanders)
//   - new + gate-level (no file) → in-progress   (amber; may be your feature)
// `strict` upgrades the amber classes to red for when the operator wants a
// hard gate. Nothing here ever blocks (advisory-only, I2).

import type { Gate, Tier } from "./gates.ts";
import type { Diag } from "./parse.ts";

export type GateStatus = "ok" | "fail" | "error" | "timeout" | "deferred";

export interface GateObs {
  gateId: string;
  status: GateStatus;
  diags: Diag[];
  tookMs: number;
  /** tail of output for fail/error gates (report detail) */
  tail: string;
}

export type ClassKind = "pre-existing" | "in-progress" | "regression";

export interface Classified {
  gateId: string;
  gateLabel: string;
  diag: Diag;
  kind: ClassKind;
}

export interface GateOutcome {
  gate: Gate;
  obs: GateObs;
  /** baseline was captured by THIS run (first observation) */
  baselinedHere: boolean;
}

export interface VerifyResult {
  tier: Tier;
  strict: boolean;
  at: number;
  tookMs: number;
  outcomes: GateOutcome[];
  regressions: Classified[];
  inProgress: Classified[];
  preExisting: Classified[];
}

/** Stable identity of a diagnostic across runs (line drift still matches). */
export function diagKey(d: Diag): string {
  if (!d.file) return `*|${d.code}|${d.message}`;
  let h = 0x811c9dc5;
  for (let i = 0; i < d.message.length; i++) {
    h ^= d.message.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return `${d.file}|${d.code}|${h.toString(16)}`;
}

/**
 * Classify one gate's observation against the permanent baseline.
 * No baseline yet → every diagnostic is pre-existing (first observation
 * defines the going-in reality).
 */
export function classifyGate(
  gate: Gate,
  obs: GateObs,
  baseline: GateObs | undefined,
  touched: ReadonlySet<string>,
  strict: boolean,
): Classified[] {
  const out: Classified[] = [];
  const baselineKeys = new Set((baseline?.diags ?? []).map(diagKey));
  for (const d of obs.diags) {
    let kind: ClassKind;
    if (!baseline || baselineKeys.has(diagKey(d))) {
      kind = "pre-existing";
    } else if (d.file === null || touched.has(d.file)) {
      kind = strict ? "regression" : "in-progress";
    } else {
      kind = "regression";
    }
    out.push({ gateId: gate.id, gateLabel: gate.label, diag: d, kind });
  }
  return out;
}

const MAX_DETAIL = 18;

function detailLines(items: Classified[]): string[] {
  const head = items.slice(0, MAX_DETAIL).map((c) => {
    const loc = c.diag.file
      ? `${short(c.diag.file)}${c.diag.line != null ? `:${c.diag.line}` : ""}`
      : `<${c.gateLabel}>`;
    return `  ${loc}  [${c.diag.code}] ${c.diag.message}`;
  });
  if (items.length > MAX_DETAIL) head.push(`  … +${items.length - MAX_DETAIL} more`);
  return head;
}

function short(p: string): string {
  return p.replace(process.cwd() + "/", "");
}

function statusGlyph(s: GateStatus): string {
  return s === "ok" ? "✓" : s === "fail" ? "✗" : s === "timeout" ? "⏱" : s === "deferred" ? "⋯" : "⚠";
}

/** Plain-text report (tool output + /verify dialog — no ANSI, dialogs are plain). */
export function renderReport(r: VerifyResult): string[] {
  const lines: string[] = [];
  const okCount = r.outcomes.filter((o) => o.obs.status === "ok").length;
  const bad = r.outcomes.filter((o) => o.obs.status !== "ok").length;
  lines.push(
    `VERIFY · ${r.tier}${r.strict ? " strict" : ""} · ${okCount}/${r.outcomes.length} gates green` +
      (bad ? ` · ${bad} not green` : ""),
  );
  const gates = r.outcomes.map((o) => `${statusGlyph(o.obs.status)} ${o.gate.label}${o.obs.tookMs ? ` ${(o.obs.tookMs / 1000).toFixed(1)}s` : ""}`);
  for (let i = 0; i < gates.length; i += 4) lines.push(gates.slice(i, i + 4).join("  ·  "));

  if (r.regressions.length > 0) {
    lines.push("");
    lines.push(`REGRESSIONS (${r.regressions.length}) — new failures in files you were not editing:`);
    lines.push(...detailLines(r.regressions));
  }
  if (r.inProgress.length > 0) {
    lines.push("");
    lines.push(`IN-PROGRESS (${r.inProgress.length}) — new, in your touched files (expected mid-work):`);
    lines.push(...detailLines(r.inProgress));
  }
  if (r.preExisting.length > 0) {
    lines.push("");
    lines.push(`PRE-EXISTING (${r.preExisting.length}) — failed at session baseline, not yours:`);
    lines.push(...detailLines(r.preExisting));
  }
  if (r.regressions.length === 0 && r.inProgress.length === 0 && r.preExisting.length === 0) {
    lines.push("");
    lines.push("clean — no diagnostics.");
  }
  const notGreen = r.outcomes.filter((o) => o.obs.status === "error" || o.obs.status === "timeout");
  if (notGreen.length > 0) {
    lines.push("");
    lines.push(`gates not run to verdict: ${notGreen.map((o) => `${o.gate.label} (${o.obs.status})`).join(", ")}`);
  }
  return lines;
}
