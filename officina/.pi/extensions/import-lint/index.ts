import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { readFileSync } from "node:fs";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { importLintConfig, jsImportedNames, pyImportedNames, renderImportNotice, unusedImports } from "./importlint.ts";

// import-lint — toolchain-free unused-import detection after every
// successful edit/write of a .py or .js/.ts file. Findings are injected as
// a compact tail message before the next LLM call; clean files cost zero
// tokens. Complements diagnostics-loop (which needs node/tsc/python).
//
// Kill switch: OFFICINA_NO_IMPORT_LINT=1 (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = importLintConfig();
  if (!cfg.enabled) return;

  const pending: string[] = [];

  pi.on("tool_result", async (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    const lang = file.endsWith(".py") ? "py" : /\.(m|c)?[jt]sx?$/.test(file) ? "js" : null;
    if (!lang) return;
    let src: string;
    try {
      src = readFileSync(file, "utf-8");
    } catch {
      return;
    }
    const names = lang === "py" ? pyImportedNames(src) : jsImportedNames(src);
    if (names.length === 0) return;
    const unused = unusedImports(src, names, lang);
    if (unused.length === 0) return;
    pending.push(renderImportNotice(file, unused, cfg.maxNames));
    emitHarnessEvent(harnessEvent("lc-imports", "unused", { detail: `${file}: ${unused.length}` }));
  });

  pi.on("context", async (event) => {
    if (pending.length === 0) return undefined;
    const block = pending.slice(0, 4).join("\n");
    pending.length = 0; // delivered once — the model's next edit re-lints
    const tail = {
      role: "custom" as const,
      customType: "lc-imports",
      content: "\n\n" + block,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}
