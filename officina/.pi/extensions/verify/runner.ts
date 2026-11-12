// verify — gate execution + baseline orchestration (exec injectable for tests).
//
// Sequential runs (deterministic widget updates, no CPU stampede), per-gate
// timeouts, optional whole-run budget for the session-start auto pass
// (deferred gates simply get baselined at their first real run — first
// observation IS the baseline). I3: the auto pass runs fire-and-forget from
// index.ts; nothing here blocks the agent loop.

import { execFile } from "node:child_process";
import type { Gate, Tier } from "./gates.ts";
import { classifyGate, type GateObs, type VerifyResult } from "./baseline.ts";
import { parseOutput, type Diag } from "./parse.ts";
import { isAbsolute, join, resolve } from "node:path";

export interface ExecResult {
  code: number;
  out: string;
  timedOut?: boolean;
  spawnErr?: string;
}

export type ExecFn = (argv: string[], cwd: string, timeoutMs: number) => Promise<ExecResult>;

export const CHEAP_TIMEOUT_MS = 180_000;
export const FULL_TIMEOUT_MS = 600_000;
export const AUTO_BUDGET_MS = 300_000;
const MAX_OUTPUT = 4_000_000;

/** Real executor: argv + cwd, combined output, hard timeout. Never throws. */
export function execGate(argv: string[], cwd: string, timeoutMs: number): Promise<ExecResult> {
  return new Promise((resolveP) => {
    execFile(
      argv[0]!,
      argv.slice(1),
      { cwd, timeout: timeoutMs, maxBuffer: MAX_OUTPUT, killSignal: "SIGKILL" },
      (err, stdout, stderr) => {
        const out = String(stdout) + String(stderr);
        if (!err) {
          resolveP({ code: 0, out });
          return;
        }
        const e = err as NodeJS.ErrnoException & { code?: number | string; killed?: boolean };
        if (e.code === "ENOENT") {
          resolveP({ code: 127, out, spawnErr: `${argv[0]}: command not found` });
          return;
        }
        if (e.killed || e.code === "ETIMEDOUT" || String(e.message).includes("SIGKILL")) {
          resolveP({ code: 124, out, timedOut: true });
          return;
        }
        resolveP({ code: typeof e.code === "number" ? e.code : 1, out });
      },
    );
  });
}

export interface RunOpts {
  tier: Tier; // "full" runs cheap+full; "cheap" runs cheap only
  strict: boolean;
  /** PERMANENT per-session first-observation baselines (mutated: first obs stored). */
  baseline: Map<string, GateObs>;
  touched: ReadonlySet<string>;
  exec?: ExecFn;
  /** live widget hook — fires after each gate */
  onGate?: (gate: Gate, obs: GateObs) => void;
  /** whole-run wall budget (auto pass); beyond it gates defer */
  budgetMs?: number;
  /** session swapped mid-run → stop BEFORE mutating the fresh baseline */
  stale?: () => boolean;
}

function classifyPool(pool: VerifyResult["regressions"]): {
  regs: VerifyResult["regressions"];
  prog: VerifyResult["regressions"];
  pre: VerifyResult["regressions"];
} {
  const regs = pool.filter((c) => c.kind === "regression");
  const prog = pool.filter((c) => c.kind === "in-progress");
  const pre = pool.filter((c) => c.kind === "pre-existing");
  return { regs, prog, pre };
}

/** Run the gate pool, classify against baselines, return the full result. */
export async function runVerify(gates: Gate[], opts: RunOpts): Promise<VerifyResult> {
  const exec = opts.exec ?? execGate;
  const started = Date.now();
  const want = gates.filter((g) => opts.tier === "full" || g.tier === "cheap");
  const outcomes: VerifyResult["outcomes"] = [];
  const pool: VerifyResult["regressions"] = [];

  for (const gate of want) {
    if (opts.stale?.()) break; // session swapped — touch nothing else
    const elapsed = Date.now() - started;
    if (opts.budgetMs !== undefined && elapsed >= opts.budgetMs) {
      const obs: GateObs = { gateId: gate.id, status: "deferred", diags: [], tookMs: 0, tail: "" };
      outcomes.push({ gate, obs, baselinedHere: false });
      opts.onGate?.(gate, obs);
      continue;
    }
    const timeout = gate.tier === "cheap" ? CHEAP_TIMEOUT_MS : FULL_TIMEOUT_MS;
    const t0 = Date.now();
    const r = await exec(gate.cmd, gate.dir, timeout);
    const tookMs = Date.now() - t0;
    if (opts.stale?.()) break; // exec outlived the session — discard this observation

    let status: GateObs["status"] = r.spawnErr ? "error" : r.timedOut ? "timeout" : r.code === 0 ? "ok" : "fail";
    let diags: Diag[] = status === "ok" || status === "error" || status === "timeout" ? [] : parseOutput(gate.parser, r.out);
    if (status === "fail" && diags.length === 0) {
      diags = [{ file: null, line: null, code: "gate", message: `${gate.label} failed (exit ${r.code})` }];
    }
    if (status === "ok" && diags.length > 0) {
      status = "fail"; // diagnostics with exit 0 still count (warnings-as-errors gates)
    }
    const tail = status === "ok" ? "" : r.out.split("\n").filter(Boolean).slice(-6).join("\n");
    const obs: GateObs = { gateId: gate.id, status, diags, tookMs, tail: (r.spawnErr ?? "") + tail };

    const hadBaseline = opts.baseline.has(gate.id);
    pool.push(...classifyGate(gate, obs, opts.baseline.get(gate.id), opts.touched, opts.strict));
    if (!hadBaseline) opts.baseline.set(gate.id, obs); // first observation defines the going-in reality
    outcomes.push({ gate, obs, baselinedHere: !hadBaseline });
    opts.onGate?.(gate, obs);
  }

  const { regs, prog, pre } = classifyPool(pool);
  return {
    tier: opts.tier,
    strict: opts.strict,
    at: started,
    tookMs: Date.now() - started,
    outcomes,
    regressions: regs,
    inProgress: prog,
    preExisting: pre,
  };
}

/** `/verify` argument parsing (pure, unit-tested). */
export function parseVerifyArgs(args: string): { tier: Tier; strict: boolean } {
  const parts = args.trim().split(/\s+/).filter(Boolean);
  const tier: Tier = parts.includes("cheap") ? "cheap" : "full";
  const strict = parts.includes("strict") || parts.includes("--strict");
  return { tier, strict };
}

/** Normalize an edited-file path to absolute (touched-set membership). */
export function normalizeTouched(p: string, cwd: string): string {
  return isAbsolute(p) ? resolve(p) : resolve(join(cwd, p));
}
