// dream-eval — the runner for the frozen eval set (2026-11-21, plan
// dream-pipeline-2026-11-21.md S1b). /dream-eval <endpoint> [model] [lora]
// scores any GGUF/adapter endpoint against the frozen set and reports.
//
// The eval set itself is seeded from the route ledger + MEMORY facts by
// buildCorpusFromLedger (S2's pure core lives here too, since seeding and
// building share types).
//
// Read-only: the runner never writes a store — it produces a report file
// under ~/.vitriol/officina/dream-eval/.
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  buildEvalSet,
  judge,
  promotionDecision,
  score,
  type EvalItem,
  type EvalReport,
  type QuestionClass,
} from "../_shared/dream-eval-core.ts";
import { readGatewayMarks } from "../_shared/gateway-health.ts";
import { seedFallbackList } from "../_shared/probe-order.ts";

const EVAL_DIR = join(homedir(), ".vitriol", "officina", "dream-eval");
const LEDGER_GLOB = "route-ledger-*.jsonl";

/** Ledger rows with outcomes (I-P3: every corpus item traces to one). */
interface LedgerRow {
  ts?: string;
  tier?: string;
  provider?: string;
  model?: string;
  reason?: string;
  outcome?: string;
  localState?: string;
  tokensIn?: number;
  tokensOut?: number;
}

export function readLedgerRows(dir: string): LedgerRow[] {
  const rows: LedgerRow[] = [];
  if (!existsSync(dir)) return rows;
  for (const f of readdirSafe(dir)) {
    if (!f.startsWith("route-ledger-") || !f.endsWith(".jsonl")) continue;
    try {
      for (const line of readFileSync(join(dir, f), "utf8").split("\n")) {
        if (!line.trim()) continue;
        try {
          rows.push(JSON.parse(line) as LedgerRow);
        } catch {
          /* skip malformed line */
        }
      }
    } catch {
      /* unreadable file — skip */
    }
  }
  return rows;
}

function readdirSafe(dir: string): string[] {
  try {
    return existsSync(dir) ? readdirSync(dir) : [];
  } catch {
    return [];
  }
}

/**
 * Seed candidates from the ledger: environment/convention facts the ledger
 * demonstrates (tier per model, profile facts). Numbers carry identity, so
 * mustContain uses them verbatim (I-P3: provenance = the row's ts+model).
 */
export function seedCandidatesFromLedger(rows: LedgerRow[]): Array<EvalItem> {
  const out: Array<EvalItem> = [];
  const byModel = new Map<string, number>();
  for (const r of rows) {
    if (!r.model || !r.ts) continue;
    byModel.set(r.model, (byModel.get(r.model) ?? 0) + 1);
  }
  for (const [model, count] of byModel) {
    if (count < 5) continue; // single-use models aren't stable facts
    out.push({
      id: `ledger-model-${count}-${model.slice(0, 20)}`,
      question: `Which models has the routing ledger served more than five turns?`,
      mustContain: [model.slice(0, Math.min(model.length, 24))],
      class: "convention",
      provenance: `route-ledger: model=${model} count=${count}`,
    });
  }
  return out;
}

async function runEval(
  base: string,
  model: string,
  lora: string | null,
  evalPath: string,
): Promise<EvalReport | string> {
  let set;
  try {
    set = JSON.parse(readFileSync(evalPath, "utf8")) as {
      hash: string;
      items: Array<EvalItem & { split: "train" | "eval" }>;
    };
  } catch (e) {
    return `cannot read eval set: ${(e as Error).message}`;
  }
  const judged = new Map<string, ReturnType<typeof judge>>();
  for (const item of set.items.filter((i) => i.split === "eval")) {
    try {
      const res = await fetch(`${base.replace(/\/$/, "")}/completion`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          prompt: item.question,
          n_predict: 200,
          temperature: 0,
          cache_prompt: false,
          ...(lora ? { lora: lora } : {}),
        }),
        signal: AbortSignal.timeout(120_000),
      });
      const doc = (await res.json()) as { content?: string };
      const answer = typeof doc.content === "string" ? doc.content : "";
      judged.set(item.id, judge(item, answer));
    } catch (e) {
      judged.set(item.id, {
        id: item.id,
        class: item.class,
        passed: false,
        reason: `request failed: ${(e as Error).message}`,
      });
    }
  }
  return score(set.items, judged, set.hash, model, lora);
}

/**
 * Score the top-N probe-ok free lanes against the frozen eval set (S4).
 * READ-ONLY over prefs and the eval set — output is a report only (I6).
 */
export async function scoreLanes(
  base: string,
  lanes: ReadonlyArray<string>,
  evalPath: string,
  now: number = Date.now(),
): Promise<Array<{ lane: string; passRate: number } | { lane: string; error: string }>> {
  const set = JSON.parse(readFileSync(evalPath, "utf8")) as {
    hash: string;
    items: Array<EvalItem & { split: "train" | "eval" }>;
  };
  const evalItems = set.items.filter((i) => i.split === "eval");
  const marks = readGatewayMarks(now).probes;
  const out: Array<{ lane: string; passRate: number } | { lane: string; error: string }> = [];
  for (const lane of lanes) {
    let passed = 0;
    let failed = false;
    for (const item of evalItems) {
      try {
        const res = await fetch(`${base.replace(/\/$/, "")}/completion`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            model: lane,
            prompt: item.question,
            n_predict: 200,
            temperature: 0,
            cache_prompt: false,
          }),
          signal: AbortSignal.timeout(120_000),
        });
        const doc = (await res.json()) as { content?: string };
        if (judge(item, typeof doc.content === "string" ? doc.content : "").passed) {
          passed++;
        }
      } catch {
        failed = true;
        break;
      }
    }
    out.push(
      failed
        ? { lane, error: "request failed" }
        : { lane, passRate: evalItems.length ? passed / evalItems.length : 0 },
    );
  }
  void marks;
  return out;
}

export default function (pi: ExtensionAPI) {
  pi.registerCommand("dream-eval", {
    description:
      "score a model/adapter against the frozen dream eval set (usage: /dream-eval [base-url] [model-id] [lora-path])",
    handler: async (args: string, ctx: unknown) => {
      const c = ctx as { ui?: any };
      const parts = args.trim().split(/\s+/).filter(Boolean);
      // S4: /dream-eval --lanes [N] [base] — score top-N probe-ok lanes.
      if (parts[0] === "--lanes") {
        const n = Math.max(1, Math.min(12, Number(parts[1]) || 4));
        const base = parts[2] ?? "http://127.0.0.1:20128";
        const evalPath = join(EVAL_DIR, "eval-set.json");
        if (!existsSync(evalPath)) {
          c.ui?.notify?.(
            "dream-eval --lanes: seed the eval set first (/dream-eval with no args after a ledger exists)",
            "warning",
          );
          return;
        }
        const marks = readGatewayMarks().probes;
        const lanes = seedFallbackList(
          Object.keys(marks),
          marks,
          n,
        ).filter((id) => marks[id]?.kind === "ok");
        if (lanes.length === 0) {
          c.ui?.notify?.(
            "dream-eval --lanes: no probe-ok lanes to score",
            "warning",
          );
          return;
        }
        c.ui?.notify?.(
          `dream-eval --lanes: scoring ${lanes.length} lanes…`,
          "info",
        );
        const results = await scoreLanes(base, lanes, evalPath);
        mkdirSync(EVAL_DIR, { recursive: true });
        const stamp = new Date().toISOString().replace(/[:.]/g, "-");
        const out = join(EVAL_DIR, `lanes-${stamp}.json`);
        writeFileSync(out, JSON.stringify({ lanes: results }, null, 2));
        const ranked = results
          .filter((r): r is { lane: string; passRate: number } => "passRate" in r)
          .sort((a, b) => b.passRate - a.passRate)
          .map((r) => `${r.lane} ${(r.passRate * 100).toFixed(0)}%`)
          .join(", ");
        c.ui?.notify?.(`dream-eval --lanes: ${ranked || "all failed"} — ${out}`, "info");
        return;
      }
      const base = parts[0] ?? "http://127.0.0.1:8279";
      const model = parts[1] ?? "local";
      const lora = parts[2] ?? null;
      const evalPath = join(EVAL_DIR, "eval-set.json");
      if (!existsSync(evalPath)) {
        // Seed from the ledger on first use (S2 preview).
        mkdirSync(EVAL_DIR, { recursive: true });
        const rows = readLedgerRows(join(homedir(), ".vitriol", "officina"));
        const candidates = seedCandidatesFromLedger(rows);
        if (candidates.length === 0) {
          c.ui?.notify?.(
            "dream-eval: no eval set and nothing to seed from the ledger",
            "warning",
          );
          return;
        }
        void buildEvalSet(candidates).then((set) => {
          writeFileSync(
            evalPath,
            JSON.stringify({ hash: set.hash, items: set.items }, null, 2),
          );
          c.ui?.notify?.(
            `dream-eval: seeded ${set.items.length} items (hash ${set.hash.slice(0, 12)}…)`,
            "info",
          );
        });
        return;
      }
      c.ui?.notify?.("dream-eval: running…", "info");
      const report = await runEval(base, model, lora, evalPath);
      if (typeof report === "string") {
        c.ui?.notify?.(`dream-eval: ${report}`, "error");
        return;
      }
      mkdirSync(EVAL_DIR, { recursive: true });
      const stamp = new Date().toISOString().replace(/[:.]/g, "-");
      const out = join(EVAL_DIR, `report-${stamp}.json`);
      writeFileSync(out, JSON.stringify(report, null, 2));
      c.ui?.notify?.(
        `dream-eval: ${(report.passRate * 100).toFixed(1)}% pass — report ${out}`,
        "info",
      );
    },
  });
}
