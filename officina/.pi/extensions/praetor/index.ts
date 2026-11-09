// praetor — first-class advisory Praetor integration.
//
// - injects `praetor instruct` once (stable block, cache-safe)
// - validates a changed file after edit/write and appends a bounded diagnostics tail
// - exposes `praetor_report` for a project report
//
// Advisory only: never blocks edits. Kill switch: OFFICINA_NO_PRAETOR=1.

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "@sinclair/typebox";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { basename, join } from "node:path";
import { tmpdir } from "node:os";
import { execFileSync } from "node:child_process";
import { injectionResult, makeDedupe } from "../_shared/inject.ts";
import {
  isPraetorSourceFile,
  parsePraetorJson,
  praetorConfig,
  renderPraetorDiagnostics,
  type PraetorDiagnostic,
} from "./praetor.ts";

const REPORT_LIMIT = 8_000;

function hasPraetor(): boolean {
  try {
    execFileSync("praetor", ["--version"], { timeout: 5_000, stdio: "pipe" });
    return true;
  } catch {
    return false;
  }
}

function readInstructions(): string {
  try {
    return String(execFileSync("praetor", ["instruct"], { timeout: 10_000, encoding: "utf8" })).trim();
  } catch {
    return "";
  }
}

function runValidate(file: string, timeoutMs: number): PraetorDiagnostic[] {
  const dir = mkdtempSync(join(tmpdir(), "officina-praetor-"));
  try {
    const target = join(dir, basename(file));
    copyFileSync(file, target);
    let stdout = "";
    try {
      stdout = execFileSync("praetor", ["validate", "--warn", "--json", "--target", dir], {
        timeout: timeoutMs,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
    } catch (error) {
      stdout = String((error as { stdout?: unknown }).stdout ?? "");
    }
    return parsePraetorJson(stdout);
  } catch {
    return [];
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function capReport(text: string): string {
  if (text.length <= REPORT_LIMIT) return text;
  return `${text.slice(0, REPORT_LIMIT)}\n\n[praetor report truncated at ${REPORT_LIMIT} chars]`;
}

export default function (pi: ExtensionAPI) {
  const cfg = praetorConfig();
  if (!cfg.enabled) return;
  if (!hasPraetor()) {
    console.warn("[praetor] binary not found; Praetor integration disabled");
    return;
  }

  const instructions = cfg.instructions ? readInstructions() : "";
  const shouldInject = makeDedupe();
  const pending = new Set<string>();

  pi.on("before_agent_start", async (event) => {
    if (!instructions) return undefined;
    if (!shouldInject(instructions)) return undefined;
    return injectionResult("officina-praetor", `\n\n${instructions}`, (event as { systemPrompt?: string }).systemPrompt ?? "");
  });

  pi.on("tool_result", async (event) => {
    const e = event as {
      toolName?: string;
      isError?: boolean;
      input?: Record<string, unknown>;
    };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (file && isPraetorSourceFile(file)) pending.add(file);
  });

  pi.on("context", async (event) => {
    if (pending.size === 0) return undefined;
    const files = [...pending];
    pending.clear();
    const diagnostics: PraetorDiagnostic[] = [];
    for (const file of files) diagnostics.push(...runValidate(file, cfg.timeoutMs));
    const block = renderPraetorDiagnostics(diagnostics, cfg.budgetChars);
    if (!block) return undefined;
    return {
      messages: [
        ...event.messages,
        {
          role: "custom" as const,
          customType: "officina-praetor",
          content: `\n\n${block}`,
          display: false,
          details: {},
          timestamp: Date.now(),
        },
      ],
    } as never;
  });

  pi.registerTool({
    name: "praetor_report",
    label: "Praetor report",
    description:
      "Run `praetor report` on a directory and return a capped markdown verification report. Use when you need a project-wide Praetor overview.",
    parameters: Type.Object({
      target: Type.Optional(Type.String({ description: "Directory to analyse (default: current workspace)" })),
    }),
    async execute(_id, params, _signal, _onUpdate, ctx) {
      const target = params.target?.trim() || (ctx as { cwd?: string }).cwd || ".";
      try {
        const out = execFileSync("praetor", ["report", "--target", target, "--format", "markdown"], {
          timeout: cfg.timeoutMs,
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
        });
        return { content: [{ type: "text" as const, text: capReport(String(out)) }], details: { target } };
      } catch (error) {
        const e = error as { stdout?: unknown; message?: unknown };
        const detail = String(e.stdout ?? e.message ?? "praetor report failed");
        return {
          content: [{ type: "text" as const, text: capReport(detail) }],
          details: { target },
          isError: true,
        };
      }
    },
  });
}
