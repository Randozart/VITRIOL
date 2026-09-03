import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync } from "node:fs";
import { couplingDisplay, loadCouplings } from "../_shared/couplings.ts";
import { fgSeq } from "../_shared/vitriolum.ts";
import { getEngineSnapshot, onEngineUpdate, startEnginePolling } from "../_shared/engine.ts";
import { RAMPS, renderGauge } from "../vitriol-decode/braille.ts";
import { fmtRate, fmtTokens } from "../vitriol-decode/decode.ts";
import { registerSidebarSection, onSidebarUpdate, renderAllSections, sidebarEnriched, sc, SIDEBAR_COLORS } from "../_shared/sidebar.ts";
import { getTaskSummary, getTaskItems } from "../task-state/index.ts";
import { getRecentTools } from "../skill-inject/index.ts";
import { getLastTopics } from "../knowledge-inject/index.ts";
import { getScratchpadSummary, getScratchpadItems } from "../scratchpad/index.ts";

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
const TEXT = fgSeq("text");
const SIDEBAR_W = 42;
// Content width inside the panel chrome: the Rust TUI sidebar's rounded
// border eats 2 columns (owner report 2026-09-02: 42-wide lines wrapped 2
// chars onto the next row). The JS split trims 1 pad col — 40 fits both.
const CONTENT_W = SIDEBAR_W - 2;
export { CONTENT_W };

// (ANSI-stripped visible width — exported for tests alongside embDiv.)
export const visibleLen = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "").length;

/** Truncate a styled string to `w` visible cells. Safe for ANSI escape sequences. */
const truncate = (s: string, w: number): string => {
  if (visibleLen(s) <= w) return s;
  let vis = 0;
  let out = "";
  let i = 0;
  while (i < s.length) {
    if (s[i] === "\x1b") {
      const rest = s.slice(i);
      const m = /^(\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b_[^\x07\x1b]*(?:\x07|\x1b\\))/.exec(rest);
      if (m) { out += m[1]; i += m[1].length; continue; }
    }
    if (vis >= w) break;
    const cp = s.codePointAt(i) ?? 0;
    const ch = String.fromCodePoint(cp);
    const cw = cp >= 0x1100 && (cp <= 0x115f || (cp >= 0x2e80 && cp <= 0xa4cf) || (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xff00 && cp <= 0xff60)) ? 2 : 1;
    if (vis + cw > w) break;
    out += ch;
    vis += cw;
    i += ch.length;
  }
  return out;
};

// Dividers (owner request 2026-09-02): every section boundary is the full
// thick rule. 2026-09-03 revision (owner): the header name lives INSIDE
// the rule — "have each divisor start with their header name inside of
// the divisor" — so a divider names the group that follows it, e.g.
// `── Plans ──────────────`. Name at the start of the rule, padded to
// exactly CONTENT_W visible columns. Exported for tests.
export const embDiv = (name: string): string => {
  const label = ` ${name} `;
  const lead = "──";
  const rest = CONTENT_W - visibleLen(lead) - visibleLen(label);
  return sc(MUTED, lead + label + "─".repeat(Math.max(0, rest)));
};

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
  // Lower priority number = rendered first (top of the sidebar).
  //
  // 2026-09-03 declutter (owner redundancy audit): the model row and the
  // session title row are GONE — the Rust header center carries the model
  // id and the header right carries the session name/id @ path. The ingest
  // row is gone — its tok/s duplicated the engine row and its cumulative
  // token count duplicated the ctx row.

  // P10: Coupling name — provider identity anchor. No model suffix: the
  // header owns the model id (owner requests 2026-09-02 + 2026-09-03).
  registerSidebarSection("coupling", 10, () => {
    const couplingFull = couplingDisplay(providerName, modelId, couplings);
    return [truncate(sc(GOLD, "◈ " + couplingFull), CONTENT_W)];
  });

  // P13: Divider with embedded group header (engine stats follow)
  registerSidebarSection("div1", 13, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Engine")];
  });

  // P20: Context usage
  registerSidebarSection("ctx", 20, () => {
    if (!ctxUsage || ctxUsage.contextWindow <= 0) return undefined;
    const pct = ctxUsage.percent;
    const g = renderGauge(RAMPS.capacity, Math.min(1, (pct ?? 0) / 100), 6);
    const filled = ctxUsage.tokens != null ? fmtTokens(ctxUsage.tokens) : "--";
    const total = fmtTokens(ctxUsage.contextWindow);
    const line = `${sc(MUTED, "ctx ")}${g} ${pct != null ? sc(SAFETY, pct.toFixed(1) + "%") : sc(MUTED, "--")} ${sc(MUTED, "· " + filled + "/" + total)}`;
    return [truncate(line, CONTENT_W)];
  });

  // P25: Engine throughput — the ONE live-rate row (2026-09-03 declutter:
  // slots and engine-boot cumulative tokens dropped as noise; prefill rate
  // row merged away — during prefill this row carries the rate).
  registerSidebarSection("engine", 25, () => {
    const eng = getEngineSnapshot();
    if (!eng.up) return undefined;
    const decoding = eng.delta.tps > 0 || eng.busy > 0;
    const g = renderGauge(
      decoding ? RAMPS.activity : RAMPS.mercury,
      decoding ? Math.max(0.08, Math.min(1, eng.delta.tps / 25)) : 0.08,
      6,
    );
    const line = `${sc(MUTED, "eng ")}${g} ${decoding ? sc(SAFETY, fmtRate(eng.delta.tps) + " tok/s") : sc(MUTED, "idle")}`;
    return [truncate(line, CONTENT_W)];
  });

  // P28: Divider with embedded group header (the Plans group follows)
  registerSidebarSection("div2", 28, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Plans")];
  });

  // P35: Task state — counts + the actual open items (owner request
  // 2026-09-02: "I'd like to see the scratchpad and todo in the sidebar").
  // Items sort open-first (in_progress, pending) via getTaskItems; cap at
  // 4 lines so the panel keeps its shape. 2026-09-03 (owner): the group
  // NAME moved into the divider (div2 → "── Plans ─…"), so the counts
  // row drops its inline prefix — the checklist starts on its own row,
  // nothing sits weirdly after the name.
  registerSidebarSection("tasks", 35, () => {
    if (!sidebarEnriched()) return undefined;
    const summary = getTaskSummary();
    if (!summary) return undefined;
    const lines: string[] = [];
    const parts: string[] = [];
    if (summary.inProgress > 0) parts.push(sc(SAFETY, `[>] ${summary.inProgress}`));
    if (summary.pending > 0) parts.push(sc(MUTED, `[ ] ${summary.pending}`));
    parts.push(sc(SAFETY, `${summary.completed} done`));
    lines.push(truncate(parts.join(sc(MUTED, " · ")), CONTENT_W));
    const items = getTaskItems().filter((t) => t.status === "in_progress" || t.status === "pending");
    for (const t of items.slice(0, 4)) {
      const mark = t.status === "in_progress" ? sc(SAFETY, "[>] ") : sc(MUTED, "[ ] ");
      lines.push(truncate("  " + mark + sc(TEXT, t.description), CONTENT_W));
    }
    return lines;
  });

  // P36: Scratchpad — summary + the actual open lines (facts, then leads;
  // owner request 2026-09-02). Cap at 4 content lines.
  registerSidebarSection("scratchpad", 36, () => {
    if (!sidebarEnriched()) return undefined;
    const s = getScratchpadSummary();
    if (!s) return undefined;
    const lines: string[] = [truncate(sc(MUTED, "note ") + sc(VIOLET, `${s.facts}f ${s.leads}l ${s.dead}d`) + sc(MUTED, ` · ${s.lines}/${s.cap}`), CONTENT_W)];
    const items = getScratchpadItems();
    if (items) {
      for (const f of items.facts.slice(0, 2)) {
        lines.push(truncate("  " + sc(VIOLET, "▪ ") + sc(TEXT, f), CONTENT_W));
      }
      for (const l of items.leads.slice(0, 2)) {
        lines.push(truncate("  " + sc(SOLVENT, "→ ") + sc(TEXT, l), CONTENT_W));
      }
    }
    return lines;
  });

  // P40: Files touched
  registerSidebarSection("files", 40, () => {
    const files = [...state.files.entries()].sort((a, b) => b[1].ts - a[1].ts).slice(0, 4);
    if (files.length === 0) return undefined;
    return [
      truncate(`${sc(MUTED, "files: ")}${files.map(([p, f]) => {
        const counts = f.add || f.del ? ` ${sc(SAFETY, "+" + f.add)} ${sc(SUBSTRATE, "−" + f.del)}` : "";
        return sc(VIOLET, shortPath(p)) + counts;
      }).join(sc(MUTED, "  "))}`, CONTENT_W),
    ];
  });

  // P42: Divider with embedded group header (session/skills follow)
  registerSidebarSection("div3", 42, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Session")];
  });

  // P45: Session spend — tokens in/out + turns (2026-09-03 declutter:
  // cwd and session id moved to the Rust header —
  // `SESSION ID: #… @ ~/path`).
  registerSidebarSection("session", 45, () => {
    return [
      truncate(`${sc(SAFETY, `↑${state.tokensIn}`)}${sc(MUTED, " ↓")}${sc(SAFETY, `${state.tokensOut}`)}${sc(MUTED, ` · ${state.turns} turns`)}`, CONTENT_W),
    ];
  });

  // P50: Recently used tools (relabeled 2026-09-03 — getRecentTools
  // returns TOOLS; "skills" was a misnomer).
  registerSidebarSection("skills", 50, () => {
    if (!sidebarEnriched()) return undefined;
    const tools = getRecentTools();
    if (tools.length === 0) return undefined;
    const shown = tools.slice(0, 5);
    return [truncate(sc(MUTED, "tools ") + shown.map((t) => sc(VIOLET, t)).join(sc(MUTED, " · ")) + sc(MUTED, ` · ${tools.length} active`), CONTENT_W)];
  });

  // P55: Knowledge refs
  registerSidebarSection("knowledge", 55, () => {
    if (!sidebarEnriched()) return undefined;
    const topics = getLastTopics();
    if (topics.length === 0) return undefined;
    const shown = topics.slice(0, 4);
    return [truncate(sc(MUTED, "ref ") + shown.map((t) => sc(SOLVENT, t)).join(sc(MUTED, " · ")) + sc(MUTED, ` · ${topics.length} injected`), CONTENT_W)];
  });

  // P58: Divider with embedded group header (command hints follow)
  registerSidebarSection("div4", 58, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Commands")];
  });

  // P90: Command hints
  registerSidebarSection("hints", 90, () => {
    return [truncate(sc(MUTED, "/help · /resume · /tree · /history · /mode"), CONTENT_W)];
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
      onEngineUpdate(() => {
        // Keep context usage fresh during ingestion (not just on message_end)
        try { ctxUsage = sessionCtx?.getContextUsage?.() ?? ctxUsage; } catch {}
        renderSidebar();
      });
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
