import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// session-panel v2 (2026-08-31, owner feedback):
//
// v1 used a ui.custom overlay for the sidebar — it captured keyboard focus
// at startup and the editor went dead. v2 renders the session panel as a
// WIDGET (setWidget), which by construction never captures input: typing
// is guaranteed to work. Panel shows: session folder, model, tokens,
// turns, session id, files touched, and the session-management keys.
// /panel toggles it; OFFICINA_SESSION_PANEL=0 kills the ext (Rule 15).
//
// /history (full transcript, scrollable overlay) stays an intentional
// MODAL — while it is open it owns the keyboard by design, q closes it.
//
// DOCKED-RIGHT sidebar (Crush/OpenCode-style layout) needs a horizontal
// layout primitive pi-tui does not have — tracked as a build item in
// docs/OFFICINA.md (vendor + fork the interactive-mode layout).

interface PanelState {
  files: Map<string, number>; // path -> last-modified timestamp (ms)
  tokensIn: number;
  tokensOut: number;
  turns: number;
}

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_SESSION_PANEL === "0") return; // Rule 15

  const state: PanelState = { files: new Map(), tokensIn: 0, tokensOut: 0, turns: 0 };
  let cwd = "";
  let modelId = "";
  let sessionId = "";
  let visible = true;
  let render = () => {};

  // Track file modifications from edit/write tool calls.
  pi.on("tool_result", (event) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const p = event.input?.file_path ?? event.input?.path;
    if (typeof p === "string") {
      state.files.set(p, Date.now());
      render();
    }
  });

  pi.on("message_end", (event) => {
    const msg = event.message as { role?: string; usage?: Record<string, number> };
    if (msg?.role !== "assistant") return;
    state.turns += 1;
    const u = msg.usage;
    if (u) {
      state.tokensIn += u.input ?? u.input_tokens ?? 0;
      state.tokensOut += u.output ?? u.output_tokens ?? u.tokens ?? 0;
    }
    render();
  });

  pi.on("model_select", (event) => {
    modelId = (event as { model?: { id?: string } }).model?.id ?? modelId;
    render();
  });

  const shortPath = (p: string) => (cwd && p.startsWith(cwd) ? p.slice(cwd.length + 1) : p);

  const panelLines = (): string[] => {
    const files = [...state.files.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6);
    const lines = [
      `◈ session · ${cwd.replace(/^\/home\/[^/]+/, "~")}  ·  ${modelId || "(default)"}  ·  ↑${state.tokensIn} ↓${state.tokensOut} · ${state.turns} turns`,
    ];
    if (files.length > 0) {
      lines.push(`   files: ${files.map(([p]) => shortPath(p)).join("  ")}`);
    }
    lines.push(`   /resume prior sessions · /tree tree · /history transcript · /panel toggle`);
    return lines;
  };

  pi.on("session_start", (_event, ctx) => {
    cwd = ctx.cwd;
    sessionId = ctx.sessionManager.getSessionId();
    modelId = ctx.model?.id ?? "";
    try {
      const title = `officina · ${cwd.replace(/^\/home\/[^/]+/, "~")}`;
      ctx.ui.setTitle?.(title);
    } catch {
      // title is decoration, never load-bearing
    }
    render = () => {
      try {
        ctx.ui.setWidget(
          "session-panel",
          visible ? panelLines() : undefined,
          { placement: "aboveEditor" },
        );
      } catch {
        // decoration must never break the session
      }
    };
    render();
  });

  pi.registerCommand("panel", {
    description: "Toggle the session panel (files, tokens, keys)",
    handler: async () => {
      visible = !visible;
      render();
    },
  });

  // ── /history — full transcript, scrollable modal overlay ─────────────
  const ESC = "\x1b";
  pi.registerCommand("history", {
    description: "Scroll the full session transcript (↑↓ pgup/pgdn, q to close)",
    handler: async (_args: string, ctx: { ui: any; sessionManager: any }) => {
      const entries = ctx.sessionManager.getEntries() ?? [];
      const msgs = entries
        .filter((e: { type?: string }) => e.type === "message")
        .map((e: { message?: unknown }) => (e as { message: unknown }).message);
      let llm: Array<{ role: string; content: unknown }> = [];
      try {
        const { convertToLlm } = await import("@earendil-works/pi-coding-agent");
        llm = convertToLlm(msgs as never[]) as typeof llm;
      } catch {
        return; // no transcript, no modal
      }
      const lines: Array<{ who: string; text: string }> = [];
      for (const m of llm) {
        if (m.role !== "user" && m.role !== "assistant") continue;
        const content = m.content;
        let text = "";
        if (typeof content === "string") text = content;
        else if (Array.isArray(content)) {
          text = content
            .filter((c): c is { type: string; text?: string } => typeof c === "object" && c !== null && "text" in c)
            .map((c) => c.text ?? "")
            .join(" ");
        }
        text = text.replace(/\s+/g, " ").trim();
        if (!text) continue;
        lines.push({ who: m.role === "user" ? "you" : "ai", text });
      }

      let offset = 0;
      const PAGE = 20;

      await ctx.ui.custom(
        (tui: any, _theme: any, _keys: any, close: () => void) => ({
          render(width: number): string[] {
            const inner = Math.max(4, width - 4);
            const header = `── session history · ${lines.length} messages · ${cwd.replace(/^\/home\/[^/]+/, "~")} ──`;
            const flat: string[] = [header, "── q close · ↑↓/pgup/pgdn · G bottom ──", ""];
            for (const l of lines) {
              const wrapped = l.text.match(new RegExp(`.{1,${inner - 4}}(\\s|$)`, "g")) ?? [l.text];
              flat.push(` ${l.who === "you" ? "you▸" : "ai ▸"} ${wrapped[0]!.trimEnd()}`);
              for (const cont of wrapped.slice(1)) flat.push(`      ${cont.trimEnd()}`);
              flat.push("");
            }
            const avail = Math.max(10, (tui?.rows ?? 40) - 8);
            const start = Math.max(0, flat.length - avail - offset);
            const view = flat.slice(start, start + avail);
            return [...view, `── [${flat.length ? Math.min(flat.length, start + avail) : 0}/${flat.length}] ──`];
          },
          invalidate() {},
          handleInput(data: string) {
            if (data === "q" || data === ESC) return close();
            if (data === `${ESC}[A`) offset = Math.max(0, offset - 1);
            else if (data === `${ESC}[B`) offset = Math.max(0, offset + 1);
            else if (data === `${ESC}[5~`) offset = Math.max(0, offset - PAGE);
            else if (data === `${ESC}[6~`) offset = Math.max(0, offset + PAGE);
            else if (data === "G") offset = 0;
            else return;
            tui.requestRender();
          },
        }),
        { overlay: true },
      );
    },
  });
}
