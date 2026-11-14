// verify-on-edit rules (2026-11-14) — pure trigger policy.
//
// The extension verifies the touched work ONCE after a settle (not per edit):
// cheap gates only, budgeted, silent when green. Invariants: I4 (at most one
// run per settle, only when the touched file set changed), I2 (failures only).

import type { VerifyResult } from "../verify/baseline.ts";

export interface VerifyOnEditConfig {
  enabled: boolean;
  budgetMs: number;
  cooldownMs: number;
  budgetChars: number;
}

export function verifyOnEditConfig(env: NodeJS.ProcessEnv = process.env): VerifyOnEditConfig {
  const num = (raw: string | undefined, fallback: number, min: number, max: number): number => {
    const n = Number(String(raw ?? "").trim());
    if (!Number.isFinite(n) || n <= 0) return fallback;
    return Math.max(min, Math.min(max, Math.trunc(n)));
  };
  return {
    enabled: env.OFFICINA_VERIFY_ON_EDIT !== "0",
    budgetMs: num(env.OFFICINA_VERIFY_BUDGET_MS, 25_000, 2_000, 300_000),
    cooldownMs: num(env.OFFICINA_VERIFY_COOLDOWN_MS, 10_000, 0, 300_000),
    budgetChars: num(env.OFFICINA_VERIFY_INJECT_CHARS, 1_200, 200, 6_000),
  };
}

/** Stable signature of the changed file set (path + content hash). */
export function signatureOf(changed: Map<string, string>): string {
  return [...changed.entries()]
    .map(([path, hash]) => `${path}:${hash}`)
    .sort()
    .join("|");
}

export interface ShouldRunInput {
  changed: Map<string, string>;
  lastSignature: string | null;
  lastRunAt: number;
  now: number;
  cooldownMs: number;
}

/** Run only when something changed since the last verification. */
export function shouldRun(input: ShouldRunInput): { run: boolean; signature: string; reason: string } {
  const signature = signatureOf(input.changed);
  if (input.changed.size === 0) {
    return { run: false, signature, reason: "nothing edited" };
  }
  if (signature === input.lastSignature) {
    return { run: false, signature, reason: "same file set as the last run" };
  }
  if (input.now - input.lastRunAt < input.cooldownMs) {
    return { run: false, signature, reason: "cooldown" };
  }
  return { run: true, signature, reason: "changed file set" };
}

export interface GateFailure {
  id: string;
  label: string;
  message: string;
}

/** Flatten regressions + in-progress diagnostics (pre-existing excluded). */
export function failingGates(result: VerifyResult): GateFailure[] {
  return [...result.regressions, ...result.inProgress].map((c) => ({
    id: c.gateId,
    label: c.gateLabel,
    message: String(c.diag?.message ?? "").slice(0, 300),
  }));
}

/** Injected tail: failures only, bounded (clean runs cost zero tokens). */
export function renderVerifyTail(failures: GateFailure[], budgetChars: number): string {
  if (failures.length === 0) return "";
  const head = `[verify: ${failures.length} gate(s) failing after your edits — fix before continuing]`;
  const trailer = "\n… [truncated; run the gate directly for the rest]";
  let out = head;
  for (const f of failures) {
    const line = `\n- ${f.id} (${f.label})${f.message ? `: ${f.message}` : ""}`;
    if (out.length + line.length + trailer.length > budgetChars) {
      out += trailer;
      break;
    }
    out += line;
  }
  return out.length > budgetChars ? `${out.slice(0, Math.max(0, budgetChars - 1))}…` : out;
}
