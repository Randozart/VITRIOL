import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";
import { getModeDef, modeColorSeq } from "../_shared/agent-mode.ts";
import { couplingDisplay, loadCouplings } from "../_shared/couplings.ts";
import { fgSeq } from "../_shared/vitriolum.ts";
import { getEngineSnapshot, onEngineUpdate, startEnginePolling } from "../_shared/engine.ts";
import { RAMPS, renderGauge } from "../vitriol-decode/braille.ts";
import { fmtRate, fmtTokens } from "../vitriol-decode/decode.ts";

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
  files: Map<string, { ts: number; add: number; del: number }>;
  tokensIn: number;
  tokensOut: number;
  turns: number;
}

// Colors: Vitriolum accents on values only (gold coupling, solvent folder,
// safety-green token flow, violet files, muted labels/frame) — palette from
// _shared/vitriolum.ts (single source, mirrors vitriol-tui/src/theme.rs).
const GOLD = fgSeq("sovereignty");
const SOLVENT = fgSeq("solvent");
const SAFETY = fgSeq("safety");
const VIOLET = fgSeq("violet");
const MUTED = fgSeq("gray");
const RESET = "\x1b[0m";
const c = (color: string, txt: string) => color + txt + RESET;
const SUBSTRATE = fgSeq("substrate");
const visibleLen = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "").length;
// Docked-sidebar width (cells): panel box renders at 40; slot gives margin
// so OfficinaSplit never wraps or clips the frame.
const SIDEBAR_W = 42;

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
  // Context usage from pi (window-aware); null tokens = unknown (honest dash).
  let ctxUsage: { tokens: number | null; contextWindow: number; percent: number | null } | null = null;
  let sessionCtx: any = null;

  const shortPath = (p: string) => (cwd && p.startsWith(cwd) ? p.slice(cwd.length + 1) : p);

  const frame = (x: string, w: number) => {
    // Frame edges follow the active mode color (registry read per render).
    const edgeSeq = modeColorSeq(getModeDef());
    const pad = Math.max(0, w - 4 - visibleLen(x));
    return `${c(edgeSeq, "│")} ${x}${" ".repeat(pad)} ${c(edgeSeq, "│")}`;
  };

  // ANSI-aware greedy word wrap: visible cells (SGR zero-width) per line <= width.
  const wrapAnsi = (s: string, width: number): string[] => {
    if (visibleLen(s) <= width) return [s];
    const words = s.split(" ");
    const lines: string[] = [];
    let cur = "";
    for (const word of words) {
      const candidate = cur ? cur + " " + word : word;
      if (visibleLen(candidate) <= width) {
        cur = candidate;
      } else {
        if (cur) lines.push(cur);
        cur = word;
      }
    }
    if (cur) lines.push(cur);
    return lines;
  };

  const panelLines = (width: number): string[] => {
    const narrow = width < 60; // docked sidebar: compact content, wrapped rows
    const w = narrow ? width - 2 : Math.min(Math.max(width - 2, 40), 160);
    const innerW = w - 4;
    const files = [...state.files.entries()].sort((a, b) => b[1].ts - a[1].ts).slice(0, 4);
    const couplingFull = couplingDisplay(providerName, modelId, couplings, modelId);
    // Narrow mode: drop the model suffix from the coupling line (it gets its
    // own wrapped row otherwise and the box has 10 rows total upstream cap).
    const coupling = narrow ? couplingFull.split(" · ")[0] : couplingFull;
    const home = cwd.replace(/^\/home\/[^/]+/, "~");
    // Mode badge (owner request 2026-09-01): registry-driven — BOLD glyph +
    // label in the mode's own color, same discipline as the agent-mode
    // widget below the editor. Configurable via ~/.vitriol/officina/modes.json.
    const modeDef = getModeDef();
    const modeSeq = modeColorSeq(modeDef);
    const badge = `${"\x1b[1m"}${modeSeq}${modeDef.glyph} ${modeDef.label}${"\x1b[0m"}`;
    // Coupling and badge on SEPARATE rows (owner bugfix 2026-09-01): the
    // combined row overflowed — ambiguous-width glyphs (▪ · –) count as 2
    // cells in pi's wrapper even though they render as 1 — and the wrapped
    // badge fragment read as duplicated text. Two short rows can never wrap.
    const badgeVisible = visibleLen(badge);
    const maxCoupling = Math.max(8, innerW - 3);
    const couplingShort = visibleLen(coupling) > maxCoupling
      ? coupling.slice(0, Math.max(1, maxCoupling - 1)) + "…"
      : coupling;
    const inner = [
      `${c(GOLD, "◈ " + couplingShort)}`,
      badge,
    ];
    // Context row: braille capacity gauge + % of window + exact filled count.
    // Honest dashes when pi can't estimate (right after compaction).
    if (ctxUsage && ctxUsage.contextWindow > 0) {
      const pct = ctxUsage.percent;
      const g = renderGauge(RAMPS.capacity, Math.min(1, (pct ?? 0) / 100), 8);
      const filled = ctxUsage.tokens != null ? fmtTokens(ctxUsage.tokens) : "--";
      inner.push(
        `${c(MUTED, "ctx ")}${g} ${pct != null ? c(SAFETY, pct + "%") : c(MUTED, "--")} ${c(MUTED, "· " + filled + " of " + fmtTokens(ctxUsage.contextWindow))}`,
      );
    }
    // Engine row: throughput truth from the shared poller (_shared/engine.ts).
    const eng = getEngineSnapshot();
    if (eng.up) {
      const busy = eng.busy;
      const total = Math.max(eng.slots.length, busy, 1);
      const decoding = eng.delta.tps > 0 || busy > 0;
      const g = renderGauge(
        decoding ? RAMPS.activity : RAMPS.mercury,
        decoding ? Math.max(0.08, Math.min(1, eng.delta.tps / 25)) : 0.08,
        8,
      );
      inner.push(
        `${c(MUTED, "eng ")}${g} ${decoding ? c(SAFETY, fmtRate(eng.delta.tps) + " tok/s") : c(MUTED, "idle")} ${c(MUTED, `· ${busy}/${total} · ${fmtTokens(eng.total)}`)}`,
      );
      // Ingestion row (owner request 2026-08-31): live prefill rate. Appears
      // only while prompt tokens are flowing — its presence IS the liveliness
      // signal. Prefill saturates compute, so the scale runs to 100 tok/s
      // (vs decode's 25) on the mercury ramp to distinguish it from decode.
      const ing = eng.ingest;
      if (ing && ing.tps > 0.5) {
        const g2 = renderGauge(RAMPS.mercury, Math.min(1, ing.tps / 100), 8);
        inner.push(
          `${c(MUTED, "ing ")}${g2} ${c(SOLVENT, fmtRate(ing.tps) + " tok/s")}${c(MUTED, ` · +${fmtTokens(ing.tokens)}`)}`,
        );
      }
    }
    inner.push(
      `${c(MUTED, "session · ")}${c(SOLVENT, home)}${c(MUTED, "  ·  ")}${c(SAFETY, `↑${state.tokensIn}`)}${c(MUTED, " ↓")}${c(SAFETY, `${state.tokensOut}`)}${c(MUTED, ` · ${state.turns} turns`)}${sessionId && !narrow ? c(MUTED, ` · ${sessionId.slice(0, 8)}`) : ""}`,
    );
    if (files.length > 0) {
      inner.push(
        `${c(MUTED, "files: ")}${files.map(([p, f]) => {
          const counts = f.add || f.del ? ` ${c(SAFETY, "+" + f.add)} ${c(SUBSTRATE, "−" + f.del)}` : "";
          return c(VIOLET, shortPath(p)) + counts;
        }).join(c(MUTED, "  "))}`,
      );
    }
    inner.push(
      narrow
        ? c(MUTED, "/history · /coupling · /panel")
        : c(MUTED, "/resume prior · /tree tree · /history transcript · /coupling swap · /panel"),
    );
    // Wrap every content row to the inner width BEFORE framing so the box
    // border always lands on the same column (frame() never wraps).
    const rows = inner.flatMap((l) => wrapAnsi(l, innerW));
    // Frame edge follows the active mode's color (owner request 2026-09-01).
    const bar = c(modeSeq, "─".repeat(w - 2));
    const corner = (ch: string) => c(modeSeq, ch);
    return [corner("╭") + bar + corner("╮"), ...rows.map((l) => frame(l, w)), corner("╰") + bar + corner("╯")];
  };

  let tui: any = null;
  const touch = () => tui?.requestRender();

  pi.on("tool_result", (event) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const p = event.input?.file_path ?? event.input?.path;
    if (typeof p !== "string") return;
    // Diff counts from the edit tool's unified patch (write: whole file added).
    let add = 0;
    let del = 0;
    const details = (event as { details?: { patch?: string; diff?: string } }).details;
    const patch = details?.patch ?? details?.diff;
    if (typeof patch === "string") {
      for (const line of patch.split("\n")) {
        if (line.startsWith("+") && !line.startsWith("+++")) add++;
        else if (line.startsWith("-") && !line.startsWith("---")) del++;
      }
    }
    const prev = state.files.get(p);
    state.files.set(p, {
      ts: Date.now(),
      add: (prev?.add ?? 0) + add,
      del: (prev?.del ?? 0) + del,
    });
    touch();
  });

  pi.on("message_end", (event) => {
    const msg = event.message as { role?: string; usage?: Record<string, number> };
    if (msg?.role !== "assistant") return;
    try {
      ctxUsage = sessionCtx?.getContextUsage?.() ?? ctxUsage;
    } catch {
      // decoration, never load-bearing
    }
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
    sessionCtx = ctx;
    try {
      ctxUsage = ctx.getContextUsage?.() ?? null;
    } catch {
      ctxUsage = null;
    }
    if (ctx.hasUI) {
      startEnginePolling();
      onEngineUpdate(() => render());
    }
    render = () => {
      try {
        // Docked layout (runtime fork): render into the sidebar column.
        // Capability probe — no env coupling; classic mode keeps the widget path.
        if (typeof (ctx.ui as any).setSidebar === "function") {
          (ctx.ui as any).setSidebar(visible ? panelLines(SIDEBAR_W) : undefined);
          return;
        }
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
