import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { registerContextState } from "../_shared/context-state.ts";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { register as registerActive } from "../_shared/active-files.ts";
import { contextChars, isLedgerMessage, LEDGER_CUSTOM_TYPE, ledgerConfig, renderLedger, type LedgerStats } from "./ledger.ts";

// session-ledger — durable orientation snapshots at run/tool boundaries.
// Never delete or relocate a previous snapshot: request-only moving tails
// force the last assistant/tool exchange to be prefetched on the next call.
// Statistics are approximate boundary observations, not live token accounting.
//
// Kill switch: OFFICINA_NO_LEDGER=1 (Rule 15).

export default function (pi: ExtensionAPI) {
  const cfg = ledgerConfig();
  if (!cfg.enabled) return;

  const files: string[] = [];
  let edits = 0;
  let block = "";

  pi.on("session_start", () => {
    files.length = 0;
    edits = 0;
    block = "";
  });

  pi.on("tool_result", async (event) => {
    const e = event as { toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError) return;
    const file = String(e.input?.path ?? e.input?.file ?? "");
    if (!file) return;
    edits++;
    registerActive(file);
    const i = files.indexOf(file);
    if (i >= 0) files.splice(i, 1);
    files.push(file);
    if (files.length > 20) files.shift(); // bounded recency window
  });

  const updateLedger = (messages: readonly unknown[]) => {
    const conversation = messages.filter((m) =>
      (m as { role?: string }).role !== "system" && !isLedgerMessage(m),
    );
    const stats: LedgerStats = {
      messages: conversation.length,
      approxTokens: Math.ceil(contextChars(conversation as Array<{ content?: unknown }>) / 4),
      files,
      edits,
    };
    const next = renderLedger(stats, cfg.maxFiles);
    if (next === block) return;
    block = next;
    emitHarnessEvent(harnessEvent("lc-ledger", "tick", { detail: block.slice(0, 200) }));
  };

  // Register refreshes first so the shared helper observes the boundary's
  // latest facts. Request-time recovery reuses this frozen snapshot.
  pi.on("before_agent_start", (_event, ctx) => {
    updateLedger(ctx.sessionManager.buildSessionProjection().messages);
  });
  pi.on("turn_end", (event) => {
    updateLedger(event.context.contextMessages);
  });
  registerContextState(pi, LEDGER_CUSTOM_TYPE, () => block ? `\n\n${block}` : "");
}
