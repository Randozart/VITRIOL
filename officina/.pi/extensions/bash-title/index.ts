import {
  createBashToolDefinition,
  type ExtensionAPI,
  type ToolRenderers,
} from "@earendil-works/pi-coding-agent";
import { Text } from "@earendil-works/pi-tui";
import { Type } from "@sinclair/typebox";
import { injectionResult, makeDedupe } from "../_shared/inject.ts";

// bash-title — one-line intent above every shell call (owner request
// 2026-11-10). The model supplies a `title` argument; this extension injects
// that contract once per session and renders it above `$ <command>` in the
// chat. The extra argument is harmless to the bash tool (its schema allows
// additional properties and execute() reads only command/timeout).
//
// Kill switch: OFFICINA_BASH_TITLES=0.

const GUIDANCE = [
  "## Bash call titles",
  "Every `bash` tool call MUST include a `title` argument: a short",
  "present-tense description of what the command does (<= 70 chars, no shell",
  "syntax). Example:",
  '{"command":"cargo test --lib","title":"Run library tests"}',
].join("\n");

function clip(text: string, max: number): string {
  const one = text.replace(/\s+/g, " ").trim();
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_BASH_TITLES === "0") return;

  // Constrained sampling builds its JSON schema from the tool parameters, so
  // a prompt-only instruction can never produce `title` for the built-in bash
  // tool (verified 2026-11-10: the model emitted only command/timeout despite
  // the injected contract). Re-register `bash` under the same name with a
  // required `title` field and delegate execution to pi's own bash
  // definition — same shell backend, output truncation, timeouts, and events.
  const bash = createBashToolDefinition(process.cwd());
  const parameters = Type.Object({
    title: Type.String({
      description:
        "Short present-tense purpose of the command (<= 70 chars, no shell syntax); shown above it in chat",
    }),
    command: Type.String({ description: "Shell command to execute" }),
    timeout: Type.Optional(
      Type.Number({ description: "Timeout in seconds (optional, no default timeout)" }),
    ),
  });
  pi.registerTool({
    name: "bash",
    label: bash.label,
    description: `${bash.description} ALWAYS include a short \`title\` describing what the command does.`,
    promptSnippet: bash.promptSnippet,
    promptGuidelines: bash.promptGuidelines,
    parameters,
    outputSchema: bash.outputSchema,
    constrainedSampling: bash.constrainedSampling,
    // Programmatic/legacy callers (codemode, nested tools) may omit `title`;
    // inject a first-line fallback before validation so required-title can
    // never turn a valid shell command into a schema error.
    prepareArguments(args) {
      const raw = (args ?? {}) as Record<string, unknown>;
      const existing = typeof raw.title === "string" ? raw.title.trim() : "";
      if (existing) return raw as never;
      const command = typeof raw.command === "string" ? raw.command : "";
      const first = (command.split("\n")[0] ?? "").trim();
      raw.title = first ? clip(first, 70) : "Run shell command";
      return raw as never;
    },
    renderCall: bash.renderCall,
    renderResult: bash.renderResult,
    async execute(id, params, signal, onUpdate, ctx) {
      return bash.execute(
        id,
        { command: params.command, timeout: params.timeout },
        signal,
        onUpdate,
        ctx,
      );
    },
  });

  let shouldInject = makeDedupe();
  let missingTitle = false;
  // /new swaps sessions in-process; the injected tail does not carry over,
  // so reset the dedupe gate or the fresh session never sees the contract.
  pi.on("session_start", () => {
    shouldInject = makeDedupe();
    missingTitle = false;
  });
  // Adaptive reminder: if the model fires a shell call without a title, make
  // sure the next turn repeats the contract instead of silently keeping the
  // bare command view.
  pi.on("tool_call", (event) => {
    if (event.toolName !== "bash" && event.toolName !== "powershell") return;
    const title = (event.input as Record<string, unknown>).title;
    if (typeof title !== "string" || title.trim().length === 0) missingTitle = true;
  });
  pi.on("before_agent_start", () => {
    if (!missingTitle && !shouldInject(GUIDANCE)) return undefined;
    missingTitle = false;
    return injectionResult("lc-bash-title", GUIDANCE);
  });

  pi.registerToolRenderer((toolName, next): ToolRenderers | undefined => {
    const base = next();
    if (!base || (toolName !== "bash" && toolName !== "powershell")) return base;
    const baseRenderCall = base.renderCall;
    if (!baseRenderCall) return base;

    return {
      ...base,
      renderCall(args, theme, context) {
        const component = baseRenderCall(args, theme, context);
        const raw = args as Record<string, unknown> | null | undefined;
        const title = typeof raw?.title === "string" ? clip(raw.title.replace(/\s+/g, " ").trim(), 80) : "";
        if (!title) return component;

        const command = typeof raw?.command === "string" ? raw.command : "";
        const timeout = typeof raw?.timeout === "number" ? theme.fg("muted", ` (timeout ${raw.timeout}s)`) : "";
        const firstLine = (command.split("\n")[0] ?? "").trim();
        const shown = context.expanded ? command : `${clip(firstLine, 120)}${command.includes("\n") ? " …" : ""}`;
        const styled =
          theme.fg("accent", theme.bold(`▸ ${title}`)) +
          "\n" +
          theme.fg("toolTitle", theme.bold(`$ ${shown}`)) +
          timeout;

        const settable = component as unknown as { setText?: (value: string) => void };
        if (typeof settable.setText === "function") {
          settable.setText(styled);
          return component;
        }
        return new Text(styled, 0, 0);
      },
    };
  });
}
