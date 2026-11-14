// verify-on-edit (2026-11-14) — cheap project gates by default, once per turn.
//
// Operator request: verification on by default. Per-edit syntax checks stay in
// diagnostics-loop; this runs the project's CHEAP gates (tsc/cargo check/
// rustfmt/ruff/go vet) after a settle, only when the touched file set changed,
// within a wall budget. Failures are injected as one bounded tail (cache-safe
// append-only) and notified; green runs are silent.
//
// Kill switch: OFFICINA_VERIFY_ON_EDIT=0.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve as resolvePath } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { detectGates } from "../verify/gates.ts";
import { runVerify } from "../verify/runner.ts";
import type { GateObs } from "../verify/baseline.ts";
import {
  failingGates,
  renderVerifyTail,
  shouldRun,
  verifyOnEditConfig,
  type GateFailure,
} from "./rules.ts";

const sha1 = (s: string) => createHash("sha1").update(s).digest("hex");

export default function (pi: ExtensionAPI) {
  const cfg = verifyOnEditConfig();
  if (!cfg.enabled) return;

  /** Touched files → content hash at the last edit. */
  const changed = new Map<string, string>();
  let lastSignature: string | null = null;
  let lastRunAt = 0;
  let running = false;
  let pendingTail = "";

  pi.on("tool_result", (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    try {
      const abs = resolvePath(process.cwd(), file);
      changed.set(abs, sha1(readFileSync(abs, "utf-8")));
    } catch {
      // vanished file — nothing to verify
    }
  });

  pi.on("agent_settled", async (_event, ctx) => {
    if (running || changed.size === 0) return;
    const decision = shouldRun({
      changed,
      lastSignature,
      lastRunAt,
      now: Date.now(),
      cooldownMs: cfg.cooldownMs,
    });
    if (!decision.run) return;
    running = true;
    try {
      const cwd = String((ctx as { cwd?: string }).cwd ?? process.cwd());
      const { gates, config } = detectGates(cwd);
      const usable = gates.filter((g) => !config.disable.includes(g.id));
      changed.clear();
      if (usable.length === 0) {
        lastSignature = decision.signature;
        lastRunAt = Date.now();
        return;
      }
      let failures: GateFailure[] = [];
      try {
        const result = await runVerify(usable, {
          tier: "cheap",
          strict: false,
          baseline: new Map<string, GateObs>(),
          touched: new Set<string>(),
          budgetMs: cfg.budgetMs,
        });
        failures = failingGates(result);
      } catch {
        failures = [];
      }
      lastSignature = decision.signature;
      lastRunAt = Date.now();
      if (failures.length === 0) return; // silent when green (I2)
      pendingTail = renderVerifyTail(failures, cfg.budgetChars);
      const ui = (ctx as { ui?: { notify?: (t: string, k?: string) => unknown } }).ui;
      try {
        void ui?.notify?.(`verify: ${failures.length} gate(s) failing after edits`, "warning");
      } catch {
        // decoration contract
      }
    } finally {
      running = false;
    }
  });

  pi.on("context", (event) => {
    if (!pendingTail) return undefined;
    const block = pendingTail;
    pendingTail = "";
    return {
      messages: [
        ...event.messages,
        {
          role: "custom" as const,
          customType: "officina-verify",
          content: `\n\n${block}`,
          display: false,
          details: {},
          timestamp: Date.now(),
        },
      ],
    } as never;
  });
}
