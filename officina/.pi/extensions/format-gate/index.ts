import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { resolveFormatterExecutable } from "../_shared/format-resolve.ts";
import { recordToolCheck } from "../_shared/toolchecks.ts";
import { formatConfig, formatterForPath, renderFormatNotice } from "./format.ts";

// format-gate — run the project's canonical formatter after every successful
// edit/write, so the model never spends tokens (or turns) matching style:
// the algorithm canonicalizes, the model is told once. Style reasoning is
// pure token waste for a small model.
//
// Notice discipline: reformats are injected ONCE as a tail message before
// the next LLM call (~budget-capped); no diffs are echoed — the next
// diagnostics-loop pass already validates the result.
//
// Kill switch: OFFICINA_NO_FORMAT=1 (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = formatConfig();
  if (!cfg.enabled) return;

  const pending: Array<{ file: string; label: string }> = [];

  pi.on("tool_result", async (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    const cmd = formatterForPath(file, resolveFormatterExecutable, {
      readText: (p) => {
        try {
          return readFileSync(p, "utf-8");
        } catch {
          return null;
        }
      },
    });
    if (!cmd) return;
    let before: string;
    try {
      before = readFileSync(file, "utf-8");
    } catch {
      return;
    }
    try {
      execFileSync(cmd.argv[0], cmd.argv.slice(1), { timeout: cfg.timeoutMs, stdio: "pipe" });
    } catch {
      return; // formatter failed — diagnostics-loop owns error reporting
    }
    let after: string;
    try {
      after = readFileSync(file, "utf-8");
    } catch {
      return;
    }
    const changed = after !== before;
    recordToolCheck(
      (e as { toolCallId?: string }).toolCallId,
      { format: { tool: cmd.label, changed } },
    );
    if (changed) {
      pending.push({ file, label: cmd.label });
      emitHarnessEvent(harnessEvent("lc-format", "reformatted", { detail: `${cmd.label} ${file}` }));
    }
  });

  pi.on("context", async (event) => {
    if (pending.length === 0) return undefined;
    const block = renderFormatNotice(pending);
    pending.length = 0; // delivered once — reformatting again would be a bug
    if (!block) return undefined;
    const tail = {
      role: "custom" as const,
      customType: "lc-format",
      content: "\n\n" + block,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}
