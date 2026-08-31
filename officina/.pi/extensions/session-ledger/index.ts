import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { contextChars, ledgerConfig, renderLedger, upsertLedger, type LedgerStats } from "./ledger.ts";

// session-ledger — one self-replacing orientation line in the context.
// Tracks files touched / edit counts from edit & write tool results, and
// computes context size from the outgoing message list each pass.
//
// Kill switch: OFFICINA_NO_LEDGER=1 (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = ledgerConfig();
  if (!cfg.enabled) return;

  const files: string[] = [];
  let edits = 0;
  let lastSession: string | undefined;

  pi.on("tool_result", async (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    edits++;
    const i = files.indexOf(file);
    if (i >= 0) files.splice(i, 1);
    files.push(file);
    if (files.length > 20) files.shift(); // bounded recency window
  });

  pi.on("context", async (event) => {
    const stats: LedgerStats = {
      messages: event.messages.length,
      approxTokens: Math.ceil(contextChars(event.messages as Array<{ content?: unknown }>) / 4),
      files,
      edits,
    };
    const block = renderLedger(stats, cfg.maxFiles);
    if (block !== lastSession) {
      lastSession = block;
      emitHarnessEvent(harnessEvent("lc-ledger", "tick", { detail: block.slice(0, 200) }));
    }
    return { messages: upsertLedger(event.messages, block) as typeof event.messages };
  });
}
