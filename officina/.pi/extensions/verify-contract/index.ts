import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { contractConfig, contractKindFor, runExternalContract, validateJson } from "./contract.ts";

// verify-contract — deterministic config-file validation after every
// successful edit/write. JSON (with duplicate-key detection) runs
// in-process; TOML/YAML delegate to python3 (opt-in via
// OFFICINA_CONTRACT_TOML/YAML=1). Errors are injected as a compact tail
// message before the next LLM call; clean files cost zero tokens.
//
// Kill switch: OFFICINA_NO_CONTRACT=1 (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = contractConfig();
  if (!cfg.enabled) return;

  const pending: Array<{ file: string; error: string }> = [];

  const pythonModules = new Map<string, boolean>();
  const havePythonModule = (m: string): boolean => {
    const cached = pythonModules.get(m);
    if (cached !== undefined) return cached;
    let ok = false;
    try {
      execFileSync("python3", ["-c", `import ${m}`], { stdio: "pipe" });
      ok = true;
    } catch {
      ok = false;
    }
    pythonModules.set(m, ok);
    return ok;
  };

  pi.on("tool_result", async (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    const kind = contractKindFor(file, cfg);
    if (!kind) return;
    let text: string;
    try {
      text = readFileSync(file, "utf-8");
    } catch {
      return;
    }
    let error: string | null;
    if (kind === "json") {
      error = validateJson(text);
    } else {
      const mod = kind === "toml" ? "tomllib" : "yaml";
      if (!havePythonModule(mod)) return;
      error = runExternalContract(kind, file, (argv) => {
        try {
          execFileSync(argv[0], argv.slice(1), { timeout: 10_000, stdio: "pipe" });
          return { ok: true, output: "" };
        } catch (err) {
          const x = err as { stderr?: string | Buffer; message?: string };
          return { ok: false, output: String(x.stderr ?? "") || String(x.message ?? "parse failed") };
        }
      });
    }
    if (error) {
      pending.push({ file, error });
      emitHarnessEvent(harnessEvent("lc-contract", "invalid", { detail: `${kind} ${file}` }));
    }
  });

  pi.on("context", async (event) => {
    if (pending.length === 0) return undefined;
    const block = pending
      .slice(0, 5)
      .map((p) => `--- ${p.file} ---\n${p.error}`)
      .join("\n");
    pending.length = 0; // delivered once — the next edit re-validates
    const tail = {
      role: "custom" as const,
      customType: "lc-contract",
      content: `\n\n[verify-contract: config file(s) failed validation — fix before continuing]\n${block}`,
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}
