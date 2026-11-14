import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/** Find the latest model-visible snapshot, including compaction's retained context. */
export function currentContextState(messages: readonly unknown[], customType: string, block: string): boolean {
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i] as { role?: string; customType?: string; content?: unknown };
    if (message.role === "custom" && message.customType === customType) {
      return message.content === block;
    }
  }
  return false;
}

/** Persist state at safe boundaries so request-only tails do not move every inference. */
export function registerContextState(pi: ExtensionAPI, customType: string, readBlock: () => string | undefined): void {
  const snapshot = (messages: readonly unknown[]) => {
    const block = readBlock();
    if (block === undefined) return;
    if (!block && !messages.some((m) => (m as { customType?: string }).customType === customType)) return;
    // Clearing state must supersede an old durable snapshot, too.
    const content = block || `[${customType}: state cleared; earlier snapshots are superseded.]`;
    return { customType, content, display: false };
  };

  pi.on("before_agent_start", (_event, ctx) => {
    const messages = ctx.sessionManager.buildSessionProjection().messages;
    const message = snapshot(messages);
    if (!message) return;
    if (currentContextState(messages, customType, message.content)) return;
    return { message };
  });

  // Boundary drafts are committed after the assistant and ALL tool results.
  // Omit `continue`: recording state must never request another model call.
  pi.on("turn_end", (event) => {
    const message = snapshot(event.context.contextMessages);
    if (!message) return;
    if (currentContextState(event.context.contextMessages, customType, message.content)) return;
    return { entries: [...event.entries, { type: "custom_message" as const, ...message }] };
  });

  // Mid-run compaction can remove a snapshot before the next safe boundary.
  // Supply it immediately, then let turn_end persist it. This recovery may
  // relocate once; ordinary continuations use the durable copy above.
  pi.on("context", (event) => {
    const message = snapshot(event.messages);
    if (!message) return;
    if (currentContextState(event.messages, customType, message.content)) return;
    return {
      messages: [...event.messages, { role: "custom" as const, ...message, timestamp: Date.now() }],
    };
  });
}
