import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { checkFor, diagConfig, renderDiagnostics, tailCap, type FileDiag } from "./diag.ts";

// diagnostics-loop — after every successful edit/write, run the file's fast
// syntax check; failures are appended as a compact tail message before the
// next LLM call (§3.3: check → auto-repair → re-check; ~300 tok budget).
// Cache-safe append-only (Rule 7). Kill switch: TRIS_NO_DIAGNOSTICS=1.
// Praetor pass opt-in (TRIS_DIAG_PRAETOR=1): single file is copied to a temp
// dir first — `--target` takes a DIRECTORY, a file target silently passes
// (AGENTS.md per-commit checklist).

export default function (pi: ExtensionAPI) {
  const cfg = diagConfig();
  if (!cfg.enabled) return;

  const pending: FileDiag[] = [];

  pi.on("tool_result", async (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    const cmd = checkFor(file, cfg);
    if (!cmd) return;
    const diag = runCheck(file, cmd);
    if (!diag.ok) pending.push(diag);
  });

  /** Run one checker; praetor sentinel routes through a temp-directory copy. */
  function runCheck(file: string, cmd: { argv: string[] }): FileDiag {
    if (cmd.argv[0] === "praetor-diag") return runPraetor(file);
    try {
      execFileSync(cmd.argv[0], cmd.argv.slice(1), { timeout: 30_000, stdio: "pipe" });
      return { file, ok: true, output: "" };
    } catch (err) {
      const e = err as { stdout?: string | Buffer; stderr?: string | Buffer; message?: string };
      const output = tailCap(String(e.stderr ?? "") + "\n" + String(e.stdout ?? ""));
      return { file, ok: false, output: output || String(e.message ?? "check failed") };
    }
  }

  /** Praetor on one file: copy to temp dir (DIRECTORY target — AGENTS.md). */
  function runPraetor(file: string): FileDiag {
    const tmp = mkdtempSync(join(tmpdir(), "tris-diag-"));
    try {
      copyFileSync(file, join(tmp, basename(file)));
      execFileSync("praetor", ["validate", "--warn", "--target", tmp], { timeout: cfg.praetorTimeoutMs, stdio: "pipe" });
      return { file, ok: true, output: "" };
    } catch (err) {
      const e = err as { stdout?: string | Buffer; message?: string };
      return { file, ok: false, output: tailCap(String(e.stdout ?? "") || String(e.message ?? "praetor failed")) };
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  }

  pi.on("context", async (event) => {
    if (pending.length === 0) return undefined;
    const block = renderDiagnostics(pending, cfg.budgetChars);
    pending.length = 0; // delivered once — the model's next edit re-checks
    if (!block) return undefined;
    const tail = {
      role: "custom" as const,
      customType: "lc-diagnostics",
      content: "\n\n" + block,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}
