import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";
import { couplingDisplay, loadCouplings } from "../_shared/couplings.ts";

// session-panel v3b (2026-08-31, owner feedback):
//
// UPSTREAM BUG FOUND: a persistent ui.custom overlay — even nonCapturing —
// breaks keyboard input routing in pi-coding-agent 0.83.0 (PTY-reproduced:
// panel on = keys dead, panel off = keys work; input never reaches the
// focused editor). Until the layout fork, the panel therefore renders as
// a WIDGET (structurally incapable of capturing input).
//
// The panel is a TOP-ANCHORED, NON-CAPTURING overlay: pinned to the top of
// the screen (anchor "top-center"), it never takes the keyboard (that bug
// cost us the editor once — typing is PTY-verified on this path).
// Contents: coupling name, session folder, token flow, turns, files
// touched, and the session keys. /panel hides/shows it.
//
// /history (full transcript, scrollable overlay) stays an intentional
// MODAL — while open it owns the keyboard by design, q closes it.
//
// Colors: Vitriolum accents on values only (gold coupling, solvent folder,
// safety-green token flow, violet files, muted labels/frame).
//
// Kill switch: OFFICINA_SESSION_PANEL=0 (Rule 15).
//
// DOCKED-RIGHT sidebar (Crush/OpenCode layout) still needs the horizontal
// layout fork tracked in docs/OFFICINA.md.

interface PanelState {
  files: Map<string, number>;
  tokensIn: number;
  tokensOut: number;
  turns: number;
}

const GOLD = "\x1b[38;2;255;215;0m";
const SOLVENT = "\x1b[38;2;0;255;255m";
const SAFETY = "\x1b[38;2;57;255;20m";
const VIOLET = "\x1b[38;2;178;148;187m";
const MUTED = "\x1b[38;2;139;148;158m";
const RESET = "\x1b[0m";
const c = (color: string, txt: string) => color + txt + RESET;
const visibleLen = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "").length;

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_SESSION_PANEL === "0") return; // Rule 15

  const state: PanelState = { files: new Map(), tokensIn: 0, tokensOut: 0, turns: 0 };
  let cwd = "";
  let modelId = "";
  let sessionId = "";
  let providerName = "llamacpp";
  let visible = true;
  let hidePanel: (() => void) | null = null;
  let showPanelFn: (() => void) | null = null;
  const couplings = loadCouplings();

  const shortPath = (p: string) => (cwd && p.startsWith(cwd) ? p.slice(cwd.length + 1) : p);

  const frame = (x: string, w: number) => {
    const pad = Math.max(0, w - 4 - visibleLen(x));
    return `│ ${x}${" ".repeat(pad)} │`;
  };

  const panelLines = (width: number): string[] => {
    const w = Math.min(Math.max(width - 2, 40), 160);
    const files = [...state.files.entries()].sort((a, b) => b[1] - a[1]).slice(0, 4);
    const coupling = couplingDisplay(providerName, modelId, couplings, modelId);
    const home = cwd.replace(/^\/home\/[^/]+/, "~");
    const inner = [
      `${c(GOLD, "◈ " + coupling)}`,
      `${c(MUTED, "session · ")}${c(SOLVENT, home)}${c(MUTED, "  ·  ")}${c(SAFETY, `↑${state.tokensIn}`)}${c(MUTED, " ↓")}${c(SAFETY, `${state.tokensOut}`)}${c(MUTED, ` · ${state.turns} turns`)}${sessionId ? c(MUTED, ` · ${sessionId.slice(0, 8)}`) : ""}`,
    ];
    if (files.length > 0) {
      inner.push(`${c(MUTED, "files: ")}${files.map(([p]) => c(VIOLET, shortPath(p))).join(c(MUTED, "  "))}`);
    }
    inner.push(c(MUTED, "/resume prior · /tree tree · /history transcript · /coupling swap · /panel"));
    const bar = "─".repeat(w - 2);
    return ["╭" + c(MUTED, bar) + "╮", ...inner.map((l) => frame(l, w)), "╰" + c(MUTED, bar) + "╯"];
  };

  let tui: any = null;
  const touch = () => tui?.requestRender();

  pi.on("tool_result", (event) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const p = event.input?.file_path ?? event.input?.path;
    if (typeof p === "string") {
      state.files.set(p, Date.now());
      touch();
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
    touch();
  });

  pi.on("model_select", (event) => {
    const m = (event as { model?: { id?: string; provider?: string } }).model;
    modelId = m?.id ?? modelId;
    providerName = m?.provider ?? providerName;
    touch();
  });

  let render = () => {};

  pi.on("session_start", (_event, ctx) => {
    cwd = ctx.cwd;
    sessionId = ctx.sessionManager.getSessionId();
    modelId = ctx.model?.id ?? "";
    try {
      ctx.ui.setTitle?.(`officina · ${cwd.replace(/^\/home\/[^/]+/, "~")}`);
    } catch {
      // decoration, never load-bearing
    }
    render = () => {
      try {
        ctx.ui.setWidget("session-panel", visible ? panelLines(process.stdout.columns ?? 100) : undefined, {
          placement: "aboveEditor",
        });
      } catch {
        // decoration must never break the session
      }
    };
    render();
  });

  pi.registerCommand("panel", {
    description: "Toggle the session panel (coupling, folder, tokens, files)",
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
            .filter((cv): cv is { type: string; text?: string } => typeof cv === "object" && cv !== null && "text" in cv)
            .map((cv) => cv.text ?? "")
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
              const wrapped = l.text.match(new RegExp(`.{1,${inner - 4}}(\s|$)`, "g")) ?? [l.text];
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
            else if (data === `${ESC}[6~`) offset = Math.max(0, offset + PAGE);
            else if (data === `${ESC}[5~`) offset = Math.max(0, offset - PAGE);
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
