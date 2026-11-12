// composer-predictions (codex-qol stage 8, 2026-11-12). One advisory
// "next step" line in the belowEditor band after each run finishes.
// Advisory only (I7): it never sends messages or mutates state beyond its
// widget; OFFICINA_NO_PREDICTIONS=1 removes it entirely. Data sources are
// files (verify-last.json) + a git status count — jiti isolation means no
// shared module state with other extensions.

import { spawnSync } from "node:child_process";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { dirtyCount, predict, readVerifyLast } from "./predict.ts";

const WIDGET_KEY = "composer-prediction";

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_PREDICTIONS === "1") return;

  let cwd = process.cwd();
  let ui: any = null;
  let lastLine = "";

  const setPrediction = (text: string | null): void => {
    const line = text ?? "";
    if (line === lastLine) return;
    lastLine = line;
    try {
      ui?.setWidget?.(WIDGET_KEY, line ? [line] : [], { placement: "belowEditor" });
    } catch {
      // decoration contract: never break the workshop
    }
  };

  const refresh = (): void => {
    const verify = readVerifyLast(join(cwd, ".officina"));
    const dirty = dirtyCount((args) => {
      try {
        const res = spawnSync("git", args, { cwd, encoding: "utf8", timeout: 5_000 });
        if (res.error || res.status !== 0) return "";
        return String(res.stdout ?? "");
      } catch {
        return "";
      }
    });
    setPrediction(predict(verify, dirty, Date.now()));
  };

  pi.on("session_start", (_event, ctx) => {
    const c = (ctx as { cwd?: string; hasUI?: boolean }).cwd;
    if (c) cwd = c;
    ui = (ctx as { hasUI?: boolean; ui?: unknown }).hasUI
      ? (ctx as { ui: any }).ui
      : null;
    lastLine = "§"; // force a clear through the churn guard
    setPrediction(null);
  });

  pi.on("agent_end", () => refresh());
}
