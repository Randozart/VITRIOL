import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { convertToLlm } from "@earendil-works/pi-coding-agent";

// session-panel (2026-08-31, owner request):
//   /history — scrollable FULL session transcript in an overlay
//     (↑/↓ line, pgup/pgdn page, g/G top/bottom, q/Esc close).
//   /panel   — toggle the right-hand sidebar: modified files this session
//     (tracked from edit/write tool calls), model, tokens, session id, cwd,
//     and the keys that matter (/resume, /tree, /history, /panel).
//
// Sessions: pi's session manager is per-project; /resume (picker) and
// /tree (navigator) are built in and advertised in the sidebar so prior
// session management is discoverable. The terminal title + widget line
// (vitriol-decode) display which folder the session belongs to.
//
// Kill switch: OFFICINA_SESSION_PANEL=0 (Rule 15).

interface PanelState {
  files: Map<string, number>; // path -> last-modified timestamp (ms)
  tokensIn: number;
  tokensOut: number;
  turns: number;
}

const WIDTH = 34;

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_SESSION_PANEL === "0") return; // Rule 15

  const state: PanelState = { files: new Map(), tokensIn: 0, tokensOut: 0, turns: 0 };
  let cwd = "";
  let modelId = "";
  let sessionId = "";
  let panelVisible = true;
  let invalidate: (() => void) | null = null;

  // Track file modifications from edit/write tool calls.
  pi.on("tool_result", (event) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const p = event.input?.file_path ?? event.input?.path;
    if (typeof p === "string") {
      state.files.set(p, Date.now());
      invalidate?.();
    }
  });

  pi.on("message_end", (event) => {
    const msg = event.message as { role?: string; usage?: { input?: number; output?: number; tokens?: number } };
    if (msg?.role !== "assistant") return;
    state.turns += 1;
    const u = msg.usage as Record<string, number> | undefined;
    if (u) {
      state.tokensIn += u.input ?? u.input_tokens ?? 0;
      state.tokensOut += u.output ?? u.output_tokens ?? u.tokens ?? 0;
    }
    invalidate?.();
  });

  pi.on("session_start", (_event, ctx) => {
    cwd = ctx.cwd;
    sessionId = ctx.sessionManager.getSessionId();
    modelId = ctx.model?.id ?? "";
    if (panelVisible) buildPanel(ctx);
  });

  pi.on("model_select", (event) => {
    modelId = (event as { model?: { id?: string } }).model?.id ?? modelId;
    invalidate?.();
  });

  const shortPath = (p: string) => (cwd && p.startsWith(cwd) ? p.slice(cwd.length + 1) : p);

  const sidebarLines = (): string[] => {
    const t = (s: string) => s;
    const files = [...state.files.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8);
    const lines = [
      t("◈ OFFICINA"),
      t(`cwd     ${cwd.replace(/^\/home\/[^/]+/, "~")}`),
      t(`model   ${modelId || "(default)"}`),
      t(`session ${sessionId.slice(0, 8)}`),
      t(`tokens  ↑${state.tokensIn} ↓${state.tokensOut} · ${state.turns} turns`),
      t(""),
      t("── files touched ──"),
    ];
    if (files.length === 0) lines.push(t("  (none yet)"));
    for (const [p] of files) lines.push(t(`  ${shortPath(p)}`));
    lines.push(t(""));
    lines.push(t("── keys ──"));
    lines.push(t("  /resume  prior sessions"));
    lines.push(t("  /tree    session tree"));
    lines.push(t("  /history full transcript"));
    lines.push(t("  /panel   toggle this sidebar"));
    return lines;
  };

  const buildPanel = (ctx: { ui: any }) => {
    ctx.ui
      .custom(
        (tui: any) => ({
          render(width: number): string[] {
            void width;
            const w = Math.min(WIDTH, Math.max(20, (tui?.columns ?? 100) - 4));
            const pad = (s: string) => {
              const cut = s.length > w - 2 ? s.slice(0, w - 5) + "…" : s;
              return cut.padEnd(w - 2);
            };
            return ["", ...sidebarLines().map(pad), ""];
          },
          invalidate() {},
          handleInput() {},
        }),
        {
          overlay: true,
          overlayOptions: { anchor: "top-right", margin: 1 } as never,
          onHandle: (handle: any) => {
            // sidebar must NOT steal focus from the editor
            try {
              handle.unfocus({ target: null });
            } catch {
              // overlay focus policy is a UI detail; fail soft
            }
            invalidate = () => {
              try {
                handle.refresh?.();
              } catch {
                // best effort
              }
            };
          },
        },
      )
      .catch(() => {
        // overlay unavailable in this mode — sidebar is decoration
      });
  };

  pi.registerCommand("panel", {
    description: "Toggle the Officina sidebar (modified files, session data)",
    handler: async (_args: string, ctx: { ui: any }) => {
      panelVisible = !panelVisible;
      if (!panelVisible) {
        invalidate?.();
        invalidate = null;
        return;
      }
      buildPanel(ctx);
    },
  });

  // ── /history — full transcript, scrollable overlay ────────────────────
  const ESC = "\x1b";
  pi.registerCommand("history", {
    description: "Scroll the full session transcript (↑↓ pgup/pgdn, q to close)",
    handler: async (_args: string, ctx: { ui: any; sessionManager: any }) => {
      const entries = ctx.sessionManager.getEntries() ?? [];
      const msgs = entries
        .filter((e: { type?: string }) => e.type === "message")
        .map((e: { message?: unknown }) => (e as { message: unknown }).message);
      const llm = convertToLlm(msgs as never[]) as Array<{ role: string; content: unknown }>;
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
        const who = m.role === "user" ? "you" : m.role === "assistant" ? "officina" : m.role;
        lines.push({ who, text });
      }

      let offset = 0; // lines from bottom
      const PAGE = 20;

      void ctx.ui.custom(
        (tui: any, _theme: any, _keys: any, close: () => void) => ({
          render(width: number): string[] {
            const inner = Math.max(4, width - 4);
            const header = `── session history · ${lines.length} messages · ${cwd.replace(/^\/home\/[^/]+/, "~")} ──`;
            const body: string[] = [header, "── q close · ↑↓/pgup/pgdn scroll ──", ""];
            const flat: string[] = [];
            for (const l of lines) {
              const wrapped = l.text.match(new RegExp(`.{1,${inner - 4}}(\\s|$)`, "g")) ?? [l.text];
              flat.push(` ${l.who === "you" ? "you▸" : "ai ▸"} ${wrapped[0]!.trimEnd()}`);
              for (const cont of wrapped.slice(1)) flat.push(`      ${cont.trimEnd()}`);
              flat.push("");
            }
            const avail = Math.max(10, (tui?.rows ?? 40) - 8);
            const start = Math.max(0, flat.length - avail - offset);
            const view = flat.slice(start, start + avail);
            body.push(...view);
            body.push(`── [${flat.length ? Math.min(flat.length, start + avail) : 0}/${flat.length}] ──`);
            return body;
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
