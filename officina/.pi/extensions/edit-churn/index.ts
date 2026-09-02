import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";
import { register as registerActive, tickTurn } from "../_shared/active-files.ts";
import { ChurnTracker, churnConfig } from "./churn.ts";

// edit-churn — detects the small-model failure mode of re-applying the same
// edit over and over. Snapshots the file's content hash at tool_call, hashes
// the result at tool_result, and feeds the (old, new) pair to the tracker.
// A directive is injected ONCE as a tail message before the next LLM call.
//
// Purely algorithmic: SHA-1 hashes and counters, zero LLM involvement.
// Kill switch: OFFICINA_NO_CHURN=1 (Rule 15).

const sha1 = (s: string) => createHash("sha1").update(s).digest("hex");

export default function (pi: ExtensionAPI) {
  const cfg = churnConfig();
  if (!cfg.enabled) return;

  const tracker = new ChurnTracker(cfg);
  /** toolCallId → pre-edit content hash (empty string when absent). */
  const snapshots = new Map<string, string>();
  const pending: string[] = [];

  const editedFile = (input: Record<string, unknown> | undefined): string =>
    String(input?.path ?? input?.file ?? "");

  pi.on("tool_call", async (event) => {
    const e = event as { toolCallId?: string; toolName?: string; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    const file = editedFile(e.input);
    if (!file || !e.toolCallId) return;
    try {
      snapshots.set(e.toolCallId, sha1(readFileSync(file, "utf-8")));
    } catch {
      snapshots.set(e.toolCallId, ""); // new file
    }
  });

  pi.on("tool_result", async (event) => {
    const e = event as { toolCallId?: string; toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError || !e.toolCallId) return;
    const oldHash = snapshots.get(e.toolCallId) ?? "";
    snapshots.delete(e.toolCallId);
    const file = editedFile(e.input);
    if (!file) return;
    let newHash: string;
    try {
      newHash = sha1(readFileSync(file, "utf-8"));
    } catch {
      return;
    }
    registerActive(file);
    const directive = tracker.record({ file, oldHash, newHash });
    if (directive) {
      pending.push(directive.message);
      emitHarnessEvent(harnessEvent("lc-churn", "loop", { detail: file }));
    }
  });

  pi.on("context", async (event) => {
    tickTurn();
    if (pending.length === 0) return undefined;
    const block = pending.join("\n\n");
    pending.length = 0; // delivered once
    const tail = {
      role: "custom" as const,
      customType: "lc-churn",
      content: "\n\n[" + block + "]",
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}
