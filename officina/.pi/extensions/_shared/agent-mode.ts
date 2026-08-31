// agent-mode shared state — one tiny module so multiple surfaces (the
// below-editor widget, the session-panel sidebar badge) agree on the
// current Plan/Build mode without polling pi or parsing UI text.
//
// Provenance: original work, this repo (First-Party Mandate).

export type AgentMode = "plan" | "build";

let current: AgentMode = "build";

export function setAgentMode(mode: AgentMode): void {
  current = mode;
}

export function getAgentMode(): AgentMode {
  return current;
}
