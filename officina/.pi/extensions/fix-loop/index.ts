// /fix-loop — bounded verify → fix → re-verify ratchet (exec-infra stage 7,
// 2026-11-12; plan .opencode/plans/officina-exec-infra-2026-11-12.md).
//
// Imports verify's PURE runner (jiti isolation forbids shared module state;
// importing files is fine). Discipline (I7): explicit invocation, N ≤ 10
// (default 3), day-capped, single-flight, ANY manual input disarms,
// OFFICINA_NO_FIX_LOOP=1 removes the command. Auto-commit after a green
// re-verify ONLY when OFFICINA_FIXLOOP_AUTOCOMMIT=1 (default off; enabled
// in this operator's env) and something is actually staged.

import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import {
  AUTOCOMMIT_MESSAGE_PREFIX,
  autocommitEnabled,
  bumpRun,
  canRun,
  fixPrompt,
  planRounds,
  type LoopState,
} from "./rules.ts";
import { detectGates } from "../verify/gates.ts";
import { runVerify } from "../verify/runner.ts";
import type { Classified, GateObs, VerifyResult } from "../verify/baseline.ts";

const STATE_PATH = join(process.cwd(), ".officina", "fixloop-state.json");

function readState(): LoopState | null {
  try {
    return JSON.parse(readFileSync(STATE_PATH, "utf8")) as LoopState;
  } catch {
    return null;
  }
}

function writeState(state: LoopState): void {
  try {
    mkdirSync(join(process.cwd(), ".officina"), { recursive: true });
    const tmp = `${STATE_PATH}.tmp-${process.pid}`;
    writeFileSync(tmp, `${JSON.stringify(state)}\n`);
    renameSync(tmp, STATE_PATH);
  } catch {
    // discipline bookkeeping is best-effort
  }
}

function autocommit(rounds: number): string {
  const status = spawnSync("git", ["status", "--porcelain"], { encoding: "utf8", timeout: 10_000 });
  if (status.status !== 0) return "autocommit skipped: not a git repo";
  if (!String(status.stdout ?? "").trim()) return "autocommit skipped: clean tree";
  const add = spawnSync("git", ["add", "-A"], { encoding: "utf8", timeout: 30_000 });
  if (add.status !== 0) return `autocommit failed: git add (${String(add.stderr ?? "").trim()})`;
  const commit = spawnSync(
    "git",
    ["commit", "-m", `${AUTOCOMMIT_MESSAGE_PREFIX} after ${rounds} round(s)`],
    { encoding: "utf8", timeout: 30_000 },
  );
  if (commit.status !== 0) return `autocommit failed: ${String(commit.stderr ?? "").trim()}`;
  return "autocommit: committed the green tree";
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_FIX_LOOP === "1") return;

  /** Non-null while a loop is in flight (single-flight marker). */
  let loop: { roundsLeft: number; max: number } | null = null;

  const verifyOnce = async (cwd: string): Promise<VerifyResult> => {
    const { gates, config } = detectGates(cwd);
    const usable = gates.filter((g) => !config.disable.includes(g.id));
    return runVerify(usable, {
      tier: "full",
      strict: false,
      baseline: new Map<string, GateObs>(),
      touched: new Set<string>(),
      budgetMs: undefined,
    });
  };

  const failures = (result: VerifyResult): Classified[] => [
    ...result.regressions,
    ...result.inProgress,
  ];

  const tick = async (ctx: {
    ui: any;
    cwd?: string;
    sendUserMessage: (text: string) => Promise<void>;
  }): Promise<void> => {
    if (!loop) return;
    const cwd = String(ctx.cwd ?? process.cwd());
    let result: VerifyResult;
    try {
      result = await verifyOnce(cwd);
    } catch (error) {
      await ctx.ui.notify?.(
        `fix-loop: verify failed (${error instanceof Error ? error.message : String(error)}) — loop stopped`,
        "warning",
      );
      loop = null;
      return;
    }
    const bad = failures(result);
    if (bad.length === 0) {
      const rounds = loop.max - loop.roundsLeft;
      const extra = autocommitEnabled(process.env) ? ` · ${autocommit(rounds)}` : "";
      await ctx.ui.notify?.(`fix-loop: GREEN after ${rounds} round(s)${extra}`, "info");
      loop = null;
      return;
    }
    if (loop.roundsLeft <= 0) {
      await ctx.ui.notify?.(
        `fix-loop: gave up — ${bad.length} gate(s) still failing after ${loop.max} round(s)`,
        "warning",
      );
      loop = null;
      return;
    }
    loop.roundsLeft -= 1;
    const round = loop.max - loop.roundsLeft;
    await ctx.sendUserMessage(fixPrompt(bad, round, loop.max));
  };

  pi.registerCommand("fix-loop", {
    description:
      "verify → fix → re-verify ratchet: /fix-loop [max rounds ≤10]. OFFICINA_FIXLOOP_AUTOCOMMIT=1 commits when green. Any manual input disarms.",
    handler: async (args: string, ctx: any) => {
      if (loop) {
        await ctx.ui.notify?.("fix-loop: already running — send any message to disarm", "warning");
        return;
      }
      const max = planRounds(args, process.env);
      if (max === 0) {
        await ctx.ui.notify?.("fix-loop: disabled (OFFICINA_NO_FIX_LOOP=1)", "warning");
        return;
      }
      const cap = canRun(readState());
      if (!cap.ok) {
        await ctx.ui.notify?.(cap.reason, "warning");
        return;
      }
      writeState(bumpRun(readState()));
      loop = { roundsLeft: max, max };
      await tick(ctx);
    },
  });

  // The fix turn ends → verify again → next round or report.
  pi.on("agent_end", (_event, ctx) => {
    if (!loop) return;
    void tick(ctx as unknown as { ui: any; cwd?: string; sendUserMessage: (t: string) => Promise<void> });
  });

  // ANY manual input disarms immediately (I7).
  pi.on("input", () => {
    loop = null;
  });
}
