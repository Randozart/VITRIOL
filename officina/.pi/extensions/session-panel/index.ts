import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
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
  cacheRead: number;
  cacheWrite: number;
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

/** Let llama-cpp-provider's session_start sync settle before warning. */
const MISMATCH_GRACE_MS = 5000;

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
/**
 * Which pi model-id strings count as aligned with the engine's loaded
 * model (2026-09-04): the alias, the loaded path, the loaded FILENAME
 * (pi's natural id — `Qwen3.8-27B-Q3_K_M.gguf`), or the stripped
 * basename. Empty modelId never matches.
 */
export function modelMatchesLoaded(
  modelId: string,
  loaded_model: string,
  loaded_path: string,
): boolean {
  if (!modelId) return false;
  const fname = loaded_path.split("/").pop() ?? "";
  const base = fname.replace(/\.gguf$/i, "");
  return (
    modelId === loaded_model
    || modelId === loaded_path
    || modelId === fname
    || modelId === base
  );
}

export const embDiv = (name: string): string => {
  const label = ` ${name} `;
  const lead = "──";
  const rest = CONTENT_W - visibleLen(lead) - visibleLen(label);
  return sc(MUTED, lead + label + "─".repeat(Math.max(0, rest)));
};

export default function (pi: ExtensionAPI) {
  if (process.env.OFFICINA_SESSION_PANEL === "0") return; // Rule 15

  const state: PanelState = { files: new Map(), tokensIn: 0, tokensOut: 0, cacheRead: 0, cacheWrite: 0, turns: 0 };
  let cwd = "";
  let modelId = "";
  let sessionId = "";
  let providerName = "llamacpp";
  let visible = true;
  const couplings = loadCouplings();
  let ctxUsage: { tokens: number | null; contextWindow: number; percent: number | null } | null = null;
  let sessionCtx: any = null;
  /** When the current engine/pi model divergence was first observed. */
  let mismatchOnset: number | null = null;
  const mismatchGraceElapsed = (): boolean =>
    mismatchOnset !== null && Date.now() - mismatchOnset >= MISMATCH_GRACE_MS;

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

  // P11: Active permission mode (ask / auto / yolo). Runtime file wins over
  // the launch env, matching permissions-guard's resolution order.
  registerSidebarSection("perm", 11, () => {
    const modePath = process.env.OFFICINA_PERMISSIONS_MODE_FILE
      || join(process.env.HOME || "", ".vitriol", "officina", "permissions-mode");
    let mode = process.env.OFFICINA_PERMISSIONS_MODE || "ask";
    try {
      if (existsSync(modePath)) {
        const raw = readFileSync(modePath, "utf8").trim().toLowerCase();
        if (raw === "ask" || raw === "auto" || raw === "yolo") mode = raw;
      }
    } catch { /* cosmetic */ }
    const color = mode === "yolo" ? VIOLET : mode === "auto" ? GOLD : SAFETY;
    return [truncate(sc(MUTED, "perm ") + sc(color, mode), CONTENT_W)];
  });

  // P12: readiness summary — one fact per line. No secrets, only presence.
  registerSidebarSection("ready", 12, () => {
    const eng = getEngineSnapshot();
    const home = process.env.HOME || "";
    const envPath = join(home, ".vitriol", "officina", "env");
    const accountsPath = join(home, ".vitriol", "officina", "accounts.json");
    const profilePath = join(home, ".vitriol", "officina", "routing-profile");
    const healthPath = join(home, ".vitriol", "officina", "provider-health.json");
    const envKeys = new Set<string>();
    try {
      if (existsSync(envPath)) {
        for (const line of readFileSync(envPath, "utf8").split(/\r?\n/)) {
          const m = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=/);
          if (m) envKeys.add(m[1]);
        }
      }
    } catch { /* cosmetic */ }
    let total = 0;
    let present = 0;
    try {
      if (existsSync(accountsPath)) {
        const doc = JSON.parse(readFileSync(accountsPath, "utf8"));
        for (const a of Array.isArray(doc?.accounts) ? doc.accounts : []) {
          const key = String(a?.credential_env ?? "").trim();
          if (!key) continue;
          total += 1;
          if (envKeys.has(key) || String(process.env[key] ?? "").trim()) present += 1;
        }
      }
    } catch { /* cosmetic */ }
    let profile = "auto";
    try { profile = readFileSync(profilePath, "utf8").trim() || "auto"; } catch { /* auto */ }
    let cooling = 0;
    try {
      const health = JSON.parse(readFileSync(healthPath, "utf8"));
      const now = Date.now();
      cooling = Object.values(health?.models ?? {}).filter((r: any) => Number(r?.cooldownUntil) > now).length;
    } catch { /* none */ }
    const lines = [
      truncate(sc(MUTED, "local ") + sc(eng.up ? SAFETY : GOLD, eng.up ? "up" : "down"), CONTENT_W),
      truncate(sc(MUTED, "cloud ") + sc(SAFETY, `${present}/${total} keys`), CONTENT_W),
      truncate(sc(MUTED, "profile ") + sc(VIOLET, profile), CONTENT_W),
    ];
    if (cooling > 0) lines.push(truncate(sc(MUTED, "cooling ") + sc(GOLD, String(cooling)), CONTENT_W));
    return lines;
  });

  // P13: Divider with embedded group header (engine stats follow)
  registerSidebarSection("div1", 13, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Engine")];
  });

  // P20: Context usage (+ the lost ejected-tokens counter, owner request
  // 2026-09-03 — old-UI parity: how much has been ejected for efficiency)
  registerSidebarSection("ctx", 20, () => {
    if (!ctxUsage || ctxUsage.contextWindow <= 0) return undefined;
    const pct = ctxUsage.percent;
    const g = renderGauge(RAMPS.capacity, Math.min(1, (pct ?? 0) / 100), 6);
    const filled = ctxUsage.tokens != null ? fmtTokens(ctxUsage.tokens) : "--";
    const total = fmtTokens(ctxUsage.contextWindow);
    // Depth is the dominant speed tax: warn before compaction gets urgent.
    const pctColor = pct != null && pct >= 90 ? VIOLET : pct != null && pct >= 75 ? GOLD : SAFETY;
    const lines = [
      truncate(`${sc(MUTED, "ctx ")}${g} ${pct != null ? sc(pctColor, pct.toFixed(1) + "%") : sc(MUTED, "--")}`, CONTENT_W),
      truncate(`${sc(MUTED, "tokens ")}${sc(SAFETY, filled)}${sc(MUTED, "/")}${sc(SAFETY, total)}`, CONTENT_W),
    ];
    if (pct != null && pct >= 75) {
      lines.push(truncate(sc(pctColor, "compact soon: /compact"), CONTENT_W));
    }
    const ejected = getEngineSnapshot().ejected;
    if (ejected > 0) {
      lines.push(truncate(`${sc(MUTED, "ejected ")}${sc(SOLVENT, fmtTokens(ejected))}`, CONTENT_W));
    }
    return lines;
  });

  // P21: Prompt-cache activity (cloud providers). Shows cumulative cache reads
  // and writes plus an approximate hit rate for the session.
  registerSidebarSection("cache", 21, () => {
    if (state.cacheRead + state.cacheWrite <= 0) return undefined;
    const prompt = state.tokensIn + state.cacheRead + state.cacheWrite;
    const hit = prompt > 0 ? (state.cacheRead / prompt) * 100 : 0;
    return [
      truncate(`${sc(MUTED, "cache read ")}${sc(SOLVENT, fmtTokens(state.cacheRead))}`, CONTENT_W),
      truncate(`${sc(MUTED, "cache write ")}${sc(VIOLET, fmtTokens(state.cacheWrite))}`, CONTENT_W),
      truncate(`${sc(MUTED, "cache hit ")}${sc(SAFETY, hit.toFixed(0) + "%")}`, CONTENT_W),
    ];
  });

  // P25: Engine throughput — the ONE live-rate row (2026-09-03 declutter:
  // slots and engine-boot cumulative tokens dropped as noise; prefill rate
  // row merged away — during prefill this row carries the rate).
  registerSidebarSection("engine", 25, () => {
    const eng = getEngineSnapshot();
    if (!eng.up) return undefined;
    // Alive-but-busy (2026-09-04): queue-backed endpoints stall while a
    // generation runs — show busy, never phantom-down.
    if (eng.stalled) {
      return [truncate(sc(SOLVENT, "⏳ engine busy — generating"), CONTENT_W)];
    }
    const decoding = eng.delta.tps > 0 || eng.busy > 0;
    const g = renderGauge(
      decoding ? RAMPS.activity : RAMPS.mercury,
      decoding ? Math.max(0.08, Math.min(1, eng.delta.tps / 25)) : 0.08,
      6,
    );
    const line = `${sc(MUTED, "eng ")}${g} ${decoding ? sc(SAFETY, fmtRate(eng.delta.tps) + " tok/s") : sc(MUTED, "idle")}`;
    const lines = [truncate(line, CONTENT_W)];
    // Boot decode total gets its OWN row (owner request 2026-09-03) —
    // appended to the rate row it wrapped uglily at 40 cols.
    if (eng.total > 0) {
      lines.push(truncate(sc(MUTED, `${fmtTokens(eng.total)} decoded this boot`), CONTENT_W));
    }
    // Distributed row-split (owner request 2026-09-09): when the engine
    // loaded RPC servers (/props rpc_servers), surface the remote host —
    // the engine's truth, not the config file (which can drift).
    if (eng.rpc.length > 0) {
      const hosts = eng.rpc.map((s) => s.split(":")[0]).join(",");
      lines.push(truncate(sc(MUTED, "rpc ") + sc(SOLVENT, hosts), CONTENT_W));
    }
    return lines;
  });

  // P26: Loaded model — the ENGINE's truth (owner request 2026-09-03):
  // alias + the underlying GGUF beside it. pi's selected model (header)
  // can drift from what the server actually loaded; when pi's string
  // matches NEITHER the alias NOR the loaded path's basename, this row
  // goes warning-orange, names both, and a one-time notice fires on onset.
  registerSidebarSection("loaded", 26, () => {
    const eng = getEngineSnapshot();
    if (!eng.up || !eng.loaded_model) return undefined;
    const base = eng.loaded_path.split("/").pop()?.replace(/\.gguf$/i, "") ?? "";
    const aligned = modelMatchesLoaded(modelId, eng.loaded_model, eng.loaded_path);
    const virtualDefault = providerName === "officina";
    if (process.env.OFFICINA_OVERFLOW === "1" || virtualDefault) {
      return [
        truncate(sc(MUTED, "loaded ") + sc(SOLVENT, eng.loaded_model), CONTENT_W),
        truncate(
          sc(MUTED, "route ") + sc(VIOLET, virtualDefault ? "Lapis Occultus" : "overflow"),
          CONTENT_W,
        ),
      ];
    }
    if (!aligned && modelId && mismatchGraceElapsed()) {
      return [
        truncate(sc(MUTED, "loaded ") + sc(SAFETY, eng.loaded_model), CONTENT_W),
        truncate(sc(MUTED, "pi ") + sc(SAFETY, modelId), CONTENT_W),
        truncate(sc(SAFETY, "⚠ model mismatch"), CONTENT_W),
      ];
    }
    const lines = [truncate(sc(MUTED, "loaded ") + sc(SOLVENT, eng.loaded_model), CONTENT_W)];
    if (base) lines.push(truncate(sc(MUTED, "file ") + sc(SOLVENT, base), CONTENT_W));
    return lines;
  });

  // P28: Divider with embedded group header (the Todo group follows)
  registerSidebarSection("div2", 28, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Todo")];
  });

  // P35: Task state — counts + the actual open items (owner request
  // 2026-09-02: "I'd like to see the scratchpad and todo in the sidebar").
  // Items sort open-first (in_progress, pending) via getTaskItems; cap at
  // 4 lines so the panel keeps its shape. 2026-09-03 (owner): the group
  // NAME moved into the divider (div2 → "── Plans ─…"), so the counts
  // row drops its inline prefix — the checklist starts on its own row,
  // nothing sits weirdly after the name.
  registerSidebarSection("tasks", 30, () => {
    if (!sidebarEnriched()) return undefined;
    const summary = getTaskSummary();
    if (!summary) return undefined;
    // One fact per line (owner request 2026-11-10: the combined counts row
    // and the 4-item preview made the list unreadable). Open items are shown
    // IN FULL, in progress first.
    const lines: string[] = [];
    if (summary.inProgress > 0) {
      lines.push(truncate(`${sc(SAFETY, "[>] ")}${sc(SAFETY, `${summary.inProgress} in progress`)}`, CONTENT_W));
    }
    if (summary.pending > 0) {
      lines.push(truncate(`${sc(MUTED, "[ ] ")}${sc(MUTED, `${summary.pending} pending`)}`, CONTENT_W));
    }
    if (summary.completed > 0) {
      lines.push(truncate(`${sc(MUTED, "✓ ")}${sc(MUTED, `${summary.completed} done`)}`, CONTENT_W));
    }
    if (summary.cancelled > 0) {
      lines.push(truncate(`${sc(MUTED, "✗ ")}${sc(MUTED, `${summary.cancelled} cancelled`)}`, CONTENT_W));
    }
    const open = getTaskItems().filter(
      (t) => t.status === "in_progress" || t.status === "pending",
    );
    const shown = open.slice(0, 6);
    for (const t of shown) {
      const mark = t.status === "in_progress" ? sc(SAFETY, "[>] ") : sc(MUTED, "[ ] ");
      lines.push(truncate("  " + mark + sc(TEXT, t.description), CONTENT_W));
    }
    if (open.length > shown.length) {
      lines.push(truncate(sc(MUTED, `  +${open.length - shown.length} open`), CONTENT_W));
    }
    return lines;
  });

  // P34: Divider with embedded group header (the Notebook follows).
  registerSidebarSection("div2b", 34, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Notebook")];
  });

  // P36: Scratchpad — counts one per line, then the actual open lines
  // (facts / context / leads, owner request 2026-11-10: full notebook, one
  // fact per line; dead ends collapse to a count).
  registerSidebarSection("scratchpad", 36, () => {
    if (!sidebarEnriched()) return undefined;
    const s = getScratchpadSummary();
    if (!s) return undefined;
    const lines: string[] = [
      truncate(sc(MUTED, "facts ") + sc(VIOLET, String(s.facts)), CONTENT_W),
      truncate(sc(MUTED, "context ") + sc(VIOLET, String(s.context)), CONTENT_W),
      truncate(sc(MUTED, "leads ") + sc(VIOLET, String(s.leads)), CONTENT_W),
      truncate(sc(MUTED, "dead ") + sc(VIOLET, String(s.dead)), CONTENT_W),
      truncate(sc(MUTED, "lines ") + sc(VIOLET, `${s.lines}/${s.cap}`), CONTENT_W),
    ];
    const items = getScratchpadItems();
    if (items) {
      const budget = 8;
      let used = 0;
      const pushItem = (prefix: string, text: string) => {
        if (used >= budget) return;
        used += 1;
        lines.push(truncate("  " + prefix + sc(TEXT, text), CONTENT_W));
      };
      for (const f of items.facts) pushItem(sc(VIOLET, "▪ "), f);
      for (const c of items.context) pushItem(sc(MUTED, "· "), c);
      for (const l of items.leads) pushItem(sc(SOLVENT, "→ "), l);
      const totalItems = items.facts.length + items.context.length + items.leads.length;
      if (totalItems > used) {
        lines.push(truncate(sc(MUTED, `  +${totalItems - used} more`), CONTENT_W));
      }
    }
    return lines;
  });

  // P40: Files touched
  registerSidebarSection("files", 40, () => {
    const files = [...state.files.entries()].sort((a, b) => b[1].ts - a[1].ts).slice(0, 5);
    if (files.length === 0) return undefined;
    return files.map(([p, f]) => {
      const counts = f.add || f.del ? ` ${sc(SAFETY, "+" + f.add)} ${sc(SUBSTRATE, "−" + f.del)}` : "";
      return truncate(sc(VIOLET, shortPath(p)) + counts, CONTENT_W);
    });
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
      truncate(sc(MUTED, "in ") + sc(SAFETY, String(state.tokensIn)), CONTENT_W),
      truncate(sc(MUTED, "out ") + sc(SAFETY, String(state.tokensOut)), CONTENT_W),
      truncate(sc(MUTED, "turns ") + sc(SAFETY, String(state.turns)), CONTENT_W),
    ];
  });

  // P50: Recently used tools (relabeled 2026-09-03 — getRecentTools
  // returns TOOLS; "skills" was a misnomer).
  registerSidebarSection("skills", 50, () => {
    if (!sidebarEnriched()) return undefined;
    const tools = getRecentTools();
    if (tools.length === 0) return undefined;
    const lines = [truncate(sc(MUTED, "tools ") + sc(SAFETY, String(tools.length)), CONTENT_W)];
    for (const t of tools.slice(0, 5)) {
      lines.push(truncate("  " + sc(VIOLET, t), CONTENT_W));
    }
    if (tools.length > 5) {
      lines.push(truncate(sc(MUTED, `  +${tools.length - 5} more`), CONTENT_W));
    }
    return lines;
  });

  // P55: Knowledge refs
  registerSidebarSection("knowledge", 55, () => {
    if (!sidebarEnriched()) return undefined;
    const topics = getLastTopics();
    if (topics.length === 0) return undefined;
    const lines = [truncate(sc(MUTED, "refs ") + sc(SAFETY, String(topics.length)), CONTENT_W)];
    for (const t of topics.slice(0, 5)) {
      lines.push(truncate("  " + sc(SOLVENT, t), CONTENT_W));
    }
    if (topics.length > 5) {
      lines.push(truncate(sc(MUTED, `  +${topics.length - 5} more`), CONTENT_W));
    }
    return lines;
  });

  // P58: Divider with embedded group header (command hints follow)
  registerSidebarSection("div4", 58, () => {
    if (!sidebarEnriched()) return undefined;
    return [embDiv("Commands")];
  });

  // P90: Command hints
  registerSidebarSection("hints", 90, () => {
    return ["/help", "/model", "/agent", "/permissions", "/cache", "/tools", "/opencode"].map((cmd) =>
      truncate(sc(MUTED, cmd), CONTENT_W),
    );
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
      state.cacheRead += u.cacheRead ?? u.cache_read ?? 0;
      state.cacheWrite += u.cacheWrite ?? u.cache_write ?? 0;
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
      let mismatchNotified = false;
      onEngineUpdate(() => {
        // Keep context usage fresh during ingestion (not just on message_end)
        try { ctxUsage = sessionCtx?.getContextUsage?.() ?? ctxUsage; } catch {}
        // One-time notice on model mismatch onset (owner request
        // 2026-09-03): the engine's loaded model vs pi's selection.
        // Uses modelMatchesLoaded (2026-09-04) — the raw !== fired on
        // aligned sessions whose id is the GGUF filename.
        //
        // Grace window (2026-11-10): llama-cpp-provider auto-syncs pi's
        // selection to the engine alias on session_start via a bounded
        // retry. The first engine poll can land BEFORE that sync settles,
        // producing a scary "engine loaded Lapis Occultus, pi selected
        // <old-gguf>" toast that immediately becomes false. Require the
        // mismatch to persist for MISMATCH_GRACE_MS first; a real drift
        // still warns.
        try {
          const eng = getEngineSnapshot();
          const overflowArmed = process.env.OFFICINA_OVERFLOW === "1";
          const virtualDefault = providerName === "officina";
          const mismatch = !overflowArmed && !virtualDefault && eng.up && !!eng.loaded_model && !!modelId
            && !modelMatchesLoaded(modelId, eng.loaded_model, eng.loaded_path);
          if (mismatch) {
            mismatchOnset ??= Date.now();
            if (!mismatchNotified && mismatchGraceElapsed()) {
              mismatchNotified = true;
              void (ctx as any)?.ui?.notify?.(
                `model mismatch: engine loaded ${eng.loaded_model}, pi selected ${modelId}`,
                "warning",
              );
            }
          } else {
            mismatchOnset = null;
            mismatchNotified = false; // re-arm on match
          }
        } catch {}
        renderSidebar();
      });
    }
    renderSidebar();
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

  // ── /import — pull another session's tasks + scratchpad into this one ───
  pi.registerCommand("import-tasks", {
    description: "Import tasks and scratchpad from another session (/import or /import <stem>)",
    handler: async (args: string, ctx: { ui: any; sessionManager: any; mode?: string }) => {
      const tasksDir = join(cwd, ".officina", "tasks");
      const scratchDir = join(cwd, ".officina");
      try {
        const { readdirSync, readFileSync, copyFileSync, existsSync } = await import("node:fs");
        const { join: j } = await import("node:path");

        // Current session stem (to exclude from list)
        const curFile = ctx.sessionManager.getSessionFile?.();
        const curStem = curFile?.split("/").pop()?.replace(/\.jsonl$/, "") ?? "";

        // Gather available sessions from task files
        let files: string[] = [];
        try { files = readdirSync(tasksDir).filter((f: string) => f.endsWith(".json")); } catch { /* no tasks dir */ }

        const sessions: Array<{ stem: string; date: string; tasks: number; hasScratch: boolean }> = [];
        for (const f of files) {
          const stem = f.replace(/\.json$/, "");
          if (stem === curStem) continue;
          let taskCount = 0;
          try {
            const data = JSON.parse(readFileSync(j(tasksDir, f), "utf8"));
            taskCount = (data.tasks ?? []).length;
          } catch { /* corrupt */ }
          const hasScratch = existsSync(j(scratchDir, `${stem}.md`));
          // Parse date from stem (format: 2026-09-05T05-11-44-395Z_...)
          const dateStr = stem.split("T")[0] ?? stem.slice(0, 10);
          sessions.push({ stem, date: dateStr, tasks: taskCount, hasScratch });
        }

        // Sort newest first
        sessions.sort((a, b) => b.stem.localeCompare(a.stem));

        if (sessions.length === 0) {
          await ctx.ui.notify?.("no other sessions found to import", "info");
          return;
        }

        const copySession = async (target: { stem: string; tasks: number; hasScratch: boolean }) => {
          const srcTasks = j(tasksDir, `${target.stem}.json`);
          const dstTasks = j(tasksDir, `${curStem}.json`);
          if (existsSync(srcTasks)) copyFileSync(srcTasks, dstTasks);
          const srcScratch = j(scratchDir, `${target.stem}.md`);
          const dstScratch = j(scratchDir, `${curStem}.md`);
          if (existsSync(srcScratch)) copyFileSync(srcScratch, dstScratch);
          await ctx.ui.notify?.(
            `imported from ${target.stem} (${target.tasks} tasks${target.hasScratch ? " + scratchpad" : ""})`,
            "info",
          );
          renderSidebar();
        };

        // If args provided, try direct import
        if (args.trim()) {
          const target = sessions.find((s) => s.stem === args.trim() || s.stem.startsWith(args.trim()));
          if (!target) {
            await ctx.ui.notify?.(`session not found: ${args.trim()}`, "warning");
            return;
          }
          await copySession(target);
          return;
        }

        // RPC has no custom-component support: use the supported select dialog
        // instead of the TUI-only overlay (2026-11-10).
        if (ctx.mode === "rpc" || typeof ctx.ui.custom !== "function") {
          const labels = sessions.map(
            (s) => `${s.date} · ${s.tasks} tasks${s.hasScratch ? " + scratch" : ""} · ${s.stem.slice(0, 24)}`,
          );
          const choice = await ctx.ui.select?.("Import session state from…", labels);
          if (choice) {
            const idx = labels.indexOf(choice);
            if (idx >= 0) await copySession(sessions[idx]);
          }
          return;
        }

        // Interactive list via custom overlay (TUI only)
        let selected = 0;
        await ctx.ui.custom(
          (tui: any, _theme: any, _keys: any, close: () => void) => ({
            render(width: number): string[] {
              const inner = Math.max(4, width - 4);
              const lines: string[] = [
                `── import session ──`,
                `── ↑↓ select · Enter import · q close ──`,
                "",
              ];
              for (let i = 0; i < sessions.length; i++) {
                const s = sessions[i];
                const mark = i === selected ? "▸ " : "  ";
                const scratch = s.hasScratch ? " + scratch" : "";
                lines.push(`${mark}${s.date}  ${s.tasks} tasks${scratch}  ${s.stem.slice(0, 12)}…`);
              }
              return lines;
            },
            invalidate() {},
            handleInput(data: string) {
              if (data === "q" || data === ESC) return close();
              if (data === `${ESC}[A`) selected = Math.max(0, selected - 1);
              else if (data === `${ESC}[B`) selected = Math.min(sessions.length - 1, selected + 1);
              else if (data === "\r" || data === "\n") {
                void copySession(sessions[selected]);
                close();
                return;
              }
              else return;
              tui.requestRender();
            },
          }),
          { overlay: true },
        );
      } catch (e) {
        await ctx.ui.notify?.(`import failed: ${(e as Error).message}`, "warning");
      }
    },
  });
}
