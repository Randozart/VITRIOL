// code-tool (2026-11-14) — the explicit "clean code" tool.
//
// Operator request: a code tool that verifies and formats by default. The
// model calls it after writing code; it formats the files (rewrites on disk),
// runs the fast syntax checks, the project's cheap gates, and Praetor, and
// returns one compact report so the NEXT edit already accounts for failures.
//
// Advisory only: nothing here blocks edits. Automatic equivalents remain in
// format-gate (format on every edit) + diagnostics-loop (syntax) +
// verify-on-edit (cheap gates per settle) + praetor (per-edit advisory).
//
// Kill switch: OFFICINA_NO_CODE_TOOL=1.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve as resolvePath } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { resolveFormatterExecutable } from "../_shared/format-resolve.ts";
import { formatterForPath } from "../format-gate/format.ts";
import { checkFor, diagConfig, tailCap } from "../diagnostics-loop/diag.ts";
import { praetorConfig, validateFile, isPraetorSourceFile } from "../praetor/praetor.ts";
import { detectGates } from "../verify/gates.ts";
import { runVerify } from "../verify/runner.ts";
import type { GateObs } from "../verify/baseline.ts";
import {
  renderReport,
  unifiedDiff,
  type CodeReport,
  type FormatOutcome,
  type GateOutcomeView,
  type PraetorOutcome,
  type SyntaxOutcome,
} from "./report.ts";

const REPORT_BUDGET = 1400;

function hasPraetor(): boolean {
  try {
    execFileSync("praetor", ["--version"], { timeout: 5_000, stdio: "pipe" });
    return true;
  } catch {
    return false;
  }
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_CODE_TOOL === "1") return;

  /** Files edited this session (absolute paths), for the default `paths`. */
  const edited = new Set<string>();

  pi.on("tool_result", (event) => {
    const e = event as { toolName?: string; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    edited.add(resolvePath(process.cwd(), file));
  });

  pi.registerTool({
    name: "code",
    label: "code",
    description:
      "Format and verify code (default: every file edited this session). Formats with the project-canonical tool, then runs syntax checks, the project's cheap gates, and Praetor. Returns a compact ✓/⚠/✗ report. Advisory; rewrites files when formatting.",
    parameters: Type.Object({
      action: Type.Optional(
        Type.Union([Type.Literal("all"), Type.Literal("format"), Type.Literal("verify")], {
          description: "all (default) = format then verify; format; verify",
        }),
      ),
      paths: Type.Optional(
        Type.Array(Type.String(), {
          description: "Files to process (default: files edited this session)",
        }),
      ),
      tier: Type.Optional(
        Type.Union([Type.Literal("cheap"), Type.Literal("full")], {
          description: "Gate tier for verify (default cheap)",
        }),
      ),
      praetor: Type.Optional(
        Type.Boolean({ description: "Include Praetor advisories (default true)" }),
      ),
    }),
    async execute(_id, params, _signal, _onUpdate, ctx) {
      const cwd = String((ctx as { cwd?: string }).cwd ?? process.cwd());
      const action = params.action ?? "all";
      const tier = params.tier ?? "cheap";
      const wantPraetor = params.praetor !== false;
      const requested = (params.paths ?? []).map((p) => resolvePath(cwd, p));
      const files = (requested.length > 0 ? requested : [...edited]).filter((f) => {
        try {
          readFileSync(f, { encoding: "utf8" });
          return true;
        } catch {
          return false;
        }
      });

      const report: CodeReport = {
        formatted: [],
        syntax: [],
        gates: { ran: false, ok: true, outcomes: [] },
        praetor: [],
        budgetBytes: REPORT_BUDGET,
      };

      if (files.length === 0) {
        return {
          content: [
            {
              type: "text" as const,
              text: "code: no files to process (pass paths or edit a file first)",
            },
          ],
          details: report,
        };
      }

      const formatOne = (file: string): FormatOutcome => {
        const before = readFileSync(file, { encoding: "utf8" });
        const cmd = formatterForPath(file, resolveFormatterExecutable, {
          readText: (p) => {
            try {
              return readFileSync(p, { encoding: "utf8" });
            } catch {
              return null;
            }
          },
        });
        if (!cmd) return { file, tool: "none", changed: false };
        try {
          execFileSync(cmd.argv[0]!, cmd.argv.slice(1), { timeout: 30_000, stdio: "pipe" });
        } catch (error) {
          return {
            file,
            tool: cmd.label,
            changed: false,
            error: error instanceof Error ? error.message : String(error),
          };
        }
        const after = readFileSync(file, { encoding: "utf8" });
        const changed = after !== before;
        return {
          file,
          tool: cmd.label,
          changed,
          diff: changed ? unifiedDiff(before, after, file) : undefined,
        };
      };

      if (action === "all" || action === "format") {
        for (const file of files) report.formatted.push(formatOne(file));
      }

      if (action === "all" || action === "verify") {
        const diagCfg = diagConfig();
        for (const file of files) {
          const cmd = checkFor(file, diagCfg);
          if (!cmd) continue;
          try {
            execFileSync(cmd.argv[0]!, cmd.argv.slice(1), { timeout: 30_000, stdio: "pipe" });
            report.syntax.push({ file, ok: true });
          } catch (error) {
            const e = error as { stdout?: unknown; stderr?: unknown; message?: string };
            const output = tailCap(String(e.stderr ?? "") + "\n" + String(e.stdout ?? ""));
            report.syntax.push({
              file,
              ok: false,
              output: output || String(e.message ?? "check failed"),
            });
          }
        }

        const { gates, config } = detectGates(cwd);
        const usable = gates.filter((g) => !config.disable.includes(g.id));
        if (usable.length > 0) {
          try {
            const result = await runVerify(usable, {
              tier,
              strict: false,
              baseline: new Map<string, GateObs>(),
              touched: new Set<string>(),
              budgetMs: tier === "cheap" ? 25_000 : 120_000,
            });
            const failed = new Set(
              [...result.regressions, ...result.inProgress].map((c) => c.gateId),
            );
            const outcomes: GateOutcomeView[] = result.outcomes.map((o) => ({
              id: o.gate.id,
              label: o.gate.label,
              ok: !failed.has(o.gate.id),
              message: failed.has(o.gate.id)
                ? String(
                    [...result.regressions, ...result.inProgress].find((c) => c.gateId === o.gate.id)
                      ?.diag?.message ?? "",
                  )
                : undefined,
            }));
            report.gates = { ran: true, ok: failed.size === 0, outcomes };
          } catch {
            report.gates = { ran: false, ok: false, outcomes: [] };
          }
        }

        const praetorCfg = praetorConfig();
        if (wantPraetor && praetorCfg.enabled && hasPraetor()) {
          const praetor: PraetorOutcome[] = [];
          for (const file of files) {
            if (!isPraetorSourceFile(file)) continue;
            for (const d of validateFile(file, praetorCfg.timeoutMs, (cmd, args, opts) =>
              execFileSync(cmd, args, opts as Parameters<typeof execFileSync>[2]),
            )) {
              praetor.push({ file: d.file, line: d.line, message: d.message });
            }
          }
          report.praetor = praetor;
        }
      }

      return {
        content: [{ type: "text" as const, text: renderReport(report) }],
        details: report,
      };
    },
  });
}
