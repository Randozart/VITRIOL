import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { emitHarnessEvent, harnessEvent } from "../_shared/events.ts";

// diff-fidelity — verifies that a "successful" edit/write actually changed
// the file. Fuzzy matchers and stale anchors can produce silent no-ops: the
// tool reports success, the model believes it, and the bug survives until
// much later when diagnosing it costs a whole debugging arc.
//
// Snapshot the content hash at tool_call; re-read at tool_result. An edit
// that reports success with a byte-identical file (or a vanished file) is
// flagged as a silent no-op and injected as a tail directive before the
// next LLM call.
//
// Purely algorithmic: one hash comparison. Kill switch: OFFICINA_NO_FIDELITY=1.

const sha1 = (s: string) => createHash("sha1").update(s).digest("hex");

interface Snapshot {
  hash: string;
  existed: boolean;
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_NO_FIDELITY === "1") return;

  /** toolCallId → pre-edit snapshot. */
  const snapshots = new Map<string, Snapshot>();
  const pending: string[] = [];

  const editedFile = (input: Record<string, unknown> | undefined): string =>
    String(input?.path ?? input?.file ?? "");

  const readHash = (file: string): { hash: string; existed: boolean } | null => {
    try {
      return { hash: sha1(readFileSync(file, "utf-8")), existed: true };
    } catch {
      return null;
    }
  };

  pi.on("tool_call", async (event) => {
    const e = event as { toolCallId?: string; toolName?: string; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    const file = editedFile(e.input);
    if (!file || !e.toolCallId) return;
    const cur = readHash(file);
    snapshots.set(e.toolCallId, { hash: cur?.hash ?? "", existed: cur !== null });
  });

  pi.on("tool_result", async (event) => {
    const e = event as { toolCallId?: string; toolName?: string; isError?: boolean; input?: Record<string, unknown> };
    const name = String(e.toolName ?? "").toLowerCase();
    if (name !== "edit" && name !== "write") return;
    if (e.isError || !e.toolCallId) return;
    const snap = snapshots.get(e.toolCallId);
    snapshots.delete(e.toolCallId);
    if (!snap) return;
    const file = editedFile(e.input);
    if (!file) return;
    const cur = readHash(file);
    let problem: string | null = null;
    if (!cur) {
      problem = "the file is MISSING after the edit reported success";
    } else if (cur.hash === snap.hash && snap.existed) {
      problem = "the on-disk content is byte-identical to before the edit";
    }
    if (!problem) return;
    pending.push(
      `diff-fidelity: your ${name} to ${file} reported success but ${problem} — ` +
        "the operation was a SILENT NO-OP. Re-read the file and retry against its exact current text.",
    );
    emitHarnessEvent(harnessEvent("lc-fidelity", "noop", { detail: `${name} ${file}` }));
  });

  pi.on("context", async (event) => {
    if (pending.length === 0) return undefined;
    const block = pending.join("\n\n");
    pending.length = 0; // delivered once
    const tail = {
      role: "custom" as const,
      customType: "lc-fidelity",
      content: "\n\n[" + block + "]",
      display: false,
      details: {},
      timestamp: Date.now(),
    };
    return { messages: [...event.messages, tail] };
  });
}
