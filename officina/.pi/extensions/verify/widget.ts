// verify — sidebar/status widget lines (ANSI — the widget surfaces parse
// truecolor SGR; see tui/src/tui/ansi.rs). Pure and unit-tested.

import { fg } from "../_shared/vitriolum.ts";
import type { VerifyResult } from "./baseline.ts";

export function renderWidgetLines(
  result: VerifyResult | null,
  running: string[],
): string[] | undefined {
  if (running.length > 0) {
    const show = running.slice(0, 3).join(", ") + (running.length > 3 ? " …" : "");
    return [fg("antidote", "◈ verify") + ` ⏳ ${fg("gray", show)}`];
  }
  if (!result) return undefined;
  const parts: string[] = [];
  const ok = result.outcomes.filter((o) => o.obs.status === "ok").length;
  parts.push(fg("safety", `✓ ${ok}`));
  if (result.regressions.length > 0) parts.push(fg("substrate", `✗ ${result.regressions.length} reg`));
  if (result.inProgress.length > 0) parts.push(fg("antidote", `⚠ ${result.inProgress.length} prog`));
  if (result.preExisting.length > 0) parts.push(fg("gray", `${result.preExisting.length} pre`));
  const broken = result.outcomes.filter((o) => o.obs.status === "error" || o.obs.status === "timeout").length;
  if (broken > 0) parts.push(fg("gray", `⏱ ${broken}`));
  const head =
    result.regressions.length > 0
      ? fg("substrate", "◈ verify")
      : result.inProgress.length > 0
        ? fg("antidote", "◈ verify")
        : fg("safety", "◈ verify");
  return [`${head}  ${parts.join("  ")}`];
}
