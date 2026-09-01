import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";
import { couplingDisplay, loadCouplings } from "../_shared/couplings.ts";
import { fgSeq } from "../_shared/vitriolum.ts";
import { getEngineSnapshot, onEngineUpdate, startEnginePolling } from "../_shared/engine.ts";
import { RAMPS, renderGauge } from "../vitriol-decode/braille.ts";
import { fmtRate, fmtTokens } from "../vitriol-decode/decode.ts";
import { registerSidebarSection, onSidebarUpdate, renderAllSections, sidebarEnriched, sc, SIDEBAR_COLORS } from "../_shared/sidebar.ts";
import { getTaskSummary } from "../task-state/index.ts";
import { getRecentTools } from "../skill-inject/index.ts";
import { getLastTopics } from "../knowledge-inject/index.ts";

// session-panel v4 (2026-09-01): sidebar section coordinator.
//
// Registers core sidebar sections (coupling, mode, ctx, engine, session,
// files, hints) and enrichment sections (title, tasks, skills, knowledge).
// Other extensions (agent-mode, vitriol-decode, etc.) register their own
// sections via the shared sidebar registry (_shared/sidebar.ts).
//
// The sidebar is rendered by collecting ALL registered sections in priority
// order and calling ctx.ui.setSidebar() with the combined output.
//
// Colors: Vitriolum accents on values only (gold coupling, solvent folder,
// safety-green token flow, violet files, muted labels/frame).
//
// Kill switch: OFFICINA_SESSION_PANEL=0 (Rule 15).

interface PanelState {
  files: Map<string, { ts: number; add: number; del: number }>;
  tokensIn: number;
  tokensOut: number;
  turns: number;
}

const GOLD = SIDEBAR_COLORS.GOLD;
const SOLVENT = SIDEBAR_COLORS.SOLVENT;
const SAFETY = SIDEBAR_COLORS.SAFETY;
const VIOLET = SIDEBAR_COLORS.VIOLET;
const MUTED = SIDEBAR_COLORS.MUTED;
const RESET = SIDEBAR_COLORS.RESET;
const SUBSTRATE = fgSeq("substrate");
const SIDEBAR_W = 42;

const visibleLen = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "").length;

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_SESSION_PANEL === "0") return; // Rule 15

  const state: PanelState = { files: new Map(), tokensIn: 0, tokensOut: 0, turns: 0 };
  let cwd = "";
  let modelId = "";
  let sessionId = "";
  let providerName = "llamacpp";
  let visible = true;
  const couplings = loadCouplings();
  let ctxUsage: { tokens: number | null; contextWindow: number; percent: number | null } | null = null;
  let sessionCtx: any = null;

  const shortPath = (p: string) => (cwd && p.startsWith(cwd) ? p.slice(cwd.length + 1) : p);

  // ANSI-aware greedy word wrap
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

  // ── Sidebar sections (priority order) ─────────────────────────────────
  // Lower priority number = rendered first (top of sidebar).

  // P10: Coupling name
  registerSidebarSection("coupling", 10, () => {
    const couplingFull = couplingDisplay(providerName, modelId, couplings, modelId);
    return [sc(GOLD, "◈ " + couplingFull)];
  });

  // P12: Session title (if named)
  registerSidebarSection("title", 12, () => {
    if (!sidebarEnriched()) return undefined;
    try {
      const name = (pi as any).getSessionName?.();
      if (!name) return undefined;
      return [sc(MUTED, `"${name}"`)];
    } catch {
      return undefined;
    }
  });

  // P20: Context usage
  registerSidebarSection("ctx", 20, () => {
    if (!ctxUsage || ctxUsage.contextWindow <= 0) return undefined;
    const pct = ctxUsage.percent;
    const g = renderGauge(RAMPS.capacity, Math.min(1, (pct ?? 0) / 100), 8);
    const filled = ctxUsage.tokens != null ? fmtTokens(ctxUsage.tokens) : "--";
    return [
      `${sc(MUTED, "ctx ")}${g} ${pct != null ? sc(SAFETY, pct + "%") : sc(MUTED, "--")} ${sc(MUTED, "· " + filled + " of " + fmtTokens(ctxUsage.contextWindow))}`,
    ];
  });

  // P25: Engine throughput
  registerSidebarSection("engine", 25, () => {
    const eng = getEngineSnapshot();
    if (!eng.up) return undefined;
    const busy = eng.busy;
    const total = Math.max(eng.slots.length, busy, 1);
    const decoding = eng.delta.tps > 0 || busy > 0;
    const g = renderGauge(
      decoding ? RAMPS.activity : RAMPS.mercury,
      decoding ? Math.max(0.08, Math.min(1, eng.delta.tps / 25)) : 0.08,
      8,
    );
    const lines = [
      `${sc(MUTED, "eng ")}${g} ${decoding ? sc(SAFETY, fmtRate(eng.delta.tps) + " tok/s") : sc(MUTED, "idle")} ${sc(MUTED, `· ${busy}/${total} · ${fmtTokens(eng.total)}`)}`,
    ];
    // Ingestion row (when active)
    const ing = eng.ingest;
    if (ing && ing.tps > 0.5) {
      const g2 = renderGauge(RAMPS.mercury, Math.min(1, ing.tps / 100), 8);
      lines.push(
        `${sc(MUTED, "ing ")}${g2} ${sc(SOLVENT, fmtRate(ing.tps) + " tok/s")}${sc(MUTED, ` · +${fmtTokens(ing.tokens)}`)}`,
      );
    }
    return lines;
  });

  // P30: Separator (when enrichment is active)
  registerSidebarSection("sep1", 30, () => {
    if (!sidebarEnriched()) return undefined;
    return [sc(MUTED, "─".repeat(40))];
  });

  // P35: Task state summary
  registerSidebarSection("tasks", 35, () => {
    if (!sidebarEnriched()) return undefined;
    const summary = getTaskSummary();
    if (!summary) return undefined;
    const parts: string[] = [];
    if (summary.inProgress > 0) parts.push(sc(SAFETY, `[>] ${summary.inProgress}`));
    if (summary.pending > 0) parts.push(sc(MUTED, `[ ] ${summary.pending}`));
    parts.push(sc(SAFETY, `${summary.completed} done`));
    return [sc(MUTED, "tasks ") + parts.join(sc(MUTED, " · "))];
  });

  // P40: Files touched
  registerSidebarSection("files", 40, () => {
    const files = [...state.files.entries()].sort((a, b) => b[1].ts - a[1].ts).slice(0, 4);
    if (files.length === 0) return undefined;
    return [
      `${sc(MUTED, "files: ")}${files.map(([p, f]) => {
        const counts = f.add || f.del ? ` ${sc(SAFETY, "+" + f.add)} ${sc(SUBSTRATE, "−" + f.del)}` : "";
        return sc(VIOLET, shortPath(p)) + counts;
      }).join(sc(MUTED, "  "))}`,
    ];
  });

  // P45: Session stats
  registerSidebarSection("session", 45, () => {
    const home = cwd.replace(/^\/home\/[^/]+/, "~");
    return [
      `${sc(MUTED, "session · ")}${sc(SOLVENT, home)}${sc(MUTED, "  ·  ")}${sc(SAFETY, `↑${state.tokensIn}`)}${sc(MUTED, " ↓")}${sc(SAFETY, `${state.tokensOut}`)}${sc(MUTED, ` · ${state.turns} turns`)}${sessionId ? sc(MUTED, ` · ${sessionId.slice(0, 8)}`) : ""}`,
    ];
  });

  // P50: Active skills
  registerSidebarSection("skills", 50, () => {
    if (!sidebarEnriched()) return undefined;
    const tools = getRecentTools();
    if (tools.length === 0) return undefined;
    const shown = tools.slice(0, 5);
    return [sc(MUTED, "skills ") + shown.map((t) => sc(VIOLET, t)).join(sc(MUTED, " · ")) + sc(MUTED, ` · ${tools.length} active`)];
  });

  // P55: Knowledge refs
  registerSidebarSection("knowledge", 55, () => {
    if (!sidebarEnriched()) return undefined;
    const topics = getLastTopics();
    if (topics.length === 0) return undefined;
    const shown = topics.slice(0, 4);
    return [sc(MUTED, "ref ") + shown.map((t) => sc(SOLVENT, t)).join(sc(MUTED, " · ")) + sc(MUTED, ` · ${topics.length} injected`)];
  });

  // P90: Command hints
  registerSidebarSection("hints", 90, () => {
    return [sc(MUTED, "/resume · /tree · /history · /mode")];
  });

  // ── Render coordinator ────────────────────────────────────────────────
  let tui: any = null;
  const touch = () => tui?.requestRender();

  // Content guard (2026-09-01, perf): engine polls every 700ms and each
  // message/tool event asks for a sidebar render, but the combined output
  // rarely changes. Skip setSidebar (which clears + rebuilds 10+ Text
  // components and invalidates the split) unless lines actually changed.
  // Measured: idle sessions drop from ~86 renders/min to ~0.
  let lastSidebarKey = "";
  const renderSidebar = () => {
    if (!sessionCtx) return;
    try {
      const lines = visible ? renderAllSections() : undefined;
      const key = lines ? lines.join("\n") : "";
      if (key === lastSidebarKey) return;
      lastSidebarKey = key;
      if (typeof (sessionCtx.ui as any).setSidebar === "function") {
        (sessionCtx.ui as any).setSidebar(lines);
        return;
      }
      // Fallback: widget mode (pre-docked layout)
      sessionCtx.ui.setWidget("session-panel", lines, {
        placement: "aboveEditor",
      });
    } catch {
      // decoration must never break the session
    }
  };

  // Re-render when any sidebar section changes.
  onSidebarUpdate(() => renderSidebar());

  // ── Event handlers ────────────────────────────────────────────────────

  pi.on("tool_result", (event) => {
    if (event.toolName !== "edit" && event.toolName !== "write") return;
    const p = event.input?.file_path ?? event.input?.path;
    if (typeof p !== "string") return;
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
      onEngineUpdate(() => renderSidebar());
    }
    renderSidebar();
  });

  pi.registerCommand("panel", {
    description: "Toggle the session panel (coupling, folder, tokens, files)",
    handler: async () => {
      visible = !visible;
      renderSidebar();
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
        return;
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
