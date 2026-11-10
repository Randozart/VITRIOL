import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// tool-gating — enforces LITTLE_CODER_ALLOWED_TOOLS at tool_call time.
//
// The subagent/plan-mode children set this env to a read-only allow-list.
// Without this extension the list was advisory only: the child could call
// edit/write/dispatch and fan out. (2026-11-10: the old repo referenced a
// "tool-gating" extension that did not exist.)

export default function (pi: ExtensionAPI) {
  const raw = process.env.LITTLE_CODER_ALLOWED_TOOLS?.trim();
  if (!raw) return;
  const allowed = new Set(
    raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  );
  if (allowed.size === 0) return;

  pi.on("tool_call", (event) => {
    const name = String((event as { toolName?: string }).toolName ?? "");
    if (!name || allowed.has(name)) return undefined;
    return {
      block: true,
      reason: `tool-gating: ${name} is not in the sub-coder allow-list`,
    };
  });
}
