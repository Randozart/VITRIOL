// Shared sidebar section registry (2026-09-01).
//
// Multiple extensions need to render into the docked sidebar (OfficinaSplit
// right column). ctx.ui.setSidebar(lines) is a FULL REPLACEMENT — only one
// caller can own it. This module provides a shared registry: each extension
// registers a named section with a render function, and the coordinator
// (session-panel) collects all sections and calls setSidebar with the combined
// output.
//
// Sections are rendered in priority order (lower = higher priority).
// Disabled sections (returning undefined) are skipped.
//
// Kill switch: OFFICINA_SIDEBAR_ENRICHED=0 disables all enriched rows.

import { fgSeq } from "./vitriolum.ts";

export type SidebarSectionFn = () => string[] | undefined;

interface SidebarSection {
  id: string;
  priority: number;
  render: SidebarSectionFn;
}

const sections = new Map<string, SidebarSection>();
const updateListeners = new Set<() => void>();

// ── Sidebar visibility tracking ──────────────────────────────────────────
// OfficinaSplit auto-hides the sidebar below MIN_COLS (100). When hidden,
// extensions that moved their content into the sidebar need to render a
// fallback in the belowEditor widget area. OfficinaSplit calls
// setSidebarVisible() on every render; extensions read isSidebarVisible()
// to decide which surface to use.
//
// Since OfficinaSplit lives in the patched vendor code (not the extensions
// directory), visibility is communicated via globalThis.
// Initial value is derived from the terminal width so narrow terminals
// (RDP/SSH sessions < 100 cols) get correct fallback behavior BEFORE the
// first OfficinaSplit render pushes the authoritative flag.
const MIN_COLS = 100;
let sidebarVisible = (process.stdout?.columns ?? 200) >= MIN_COLS;

/** Called by OfficinaSplit after every render with the actual visibility. */
export function setSidebarVisible(visible: boolean): void {
  sidebarVisible = visible;
}

/** True when the sidebar column is on screen (terminal >= 100 cols). */
export function isSidebarVisible(): boolean {
  return sidebarVisible;
}

// Bridge for the patched vendor code (OfficinaSplit) to push visibility.
(globalThis as any).__officinaSidebarVisible = (v: boolean) => setSidebarVisible(v);

// ── belowEditor fallback helper ──────────────────────────────────────────
// Extensions that moved content to the sidebar need a belowEditor fallback
// for narrow terminals. This helper tracks the last-set state so we only
// call setWidget when the state actually changes (avoids invalidating the
// widget container on every engine poll tick, which disrupts scrolling).

/** Create a stateful belowEditor fallback that only calls setWidget on changes. */
export function createBelowEditorFallback(
  widgetKey: string,
  getLines: () => string[] | undefined,
): (ui: any) => void {
  let lastVisible: boolean | null = null;
  let lastLines: string | null = null;
  return (ui: any) => {
    if (!ui) return;
    const visible = isSidebarVisible();
    const lines = visible ? undefined : getLines();
    const linesKey = lines ? JSON.stringify(lines) : "";
    if (visible === lastVisible && linesKey === lastLines) return;
    lastVisible = visible;
    lastLines = linesKey;
    try {
      ui.setWidget(widgetKey, lines, { placement: "belowEditor" });
    } catch { /* decoration */ }
  };
}

/** Register a sidebar section. Overwrites if id already exists. */
export function registerSidebarSection(id: string, priority: number, render: SidebarSectionFn): void {
  sections.set(id, { id, priority, render });
  notifyUpdate();
}

/** Unregister a sidebar section (e.g. on disable). */
export function unregisterSidebarSection(id: string): void {
  sections.delete(id);
  notifyUpdate();
}

/** Request a re-render of the sidebar (call after section content changes). */
export function requestSidebarUpdate(): void {
  notifyUpdate();
}

/** Register a callback that fires when any section changes. Multiple
 *  listeners are supported (session-panel + extension fallbacks). */
export function onSidebarUpdate(fn: () => void): () => void {
  updateListeners.add(fn);
  return () => { updateListeners.delete(fn); };
}

function notifyUpdate(): void {
  for (const fn of [...updateListeners]) {
    try { fn(); } catch { /* never break the registry */ }
  }
}

/** Render all registered sections in priority order. Returns the combined
 *  line array ready for ctx.ui.setSidebar(). */
export function renderAllSections(): string[] {
  const sorted = [...sections.values()].sort((a, b) => a.priority - b.priority);
  const lines: string[] = [];
  for (const s of sorted) {
    try {
      const result = s.render();
      if (result && result.length > 0) {
        lines.push(...result);
      }
    } catch {
      // a broken section must never take down the sidebar
    }
  }
  return lines;
}

/** True if enriched sidebar rows are enabled (default: true). */
export function sidebarEnriched(): boolean {
  return process.env.OFFICINA_SIDEBAR_ENRICHED !== "0";
}

// ── Shared color constants for sidebar sections ──────────────────────────
const GOLD = fgSeq("sovereignty");
const SOLVENT = fgSeq("solvent");
const SAFETY = fgSeq("safety");
const VIOLET = fgSeq("violet");
const MUTED = fgSeq("gray");
const RESET = "\x1b[0m";
export const SIDEBAR_COLORS = { GOLD, SOLVENT, SAFETY, VIOLET, MUTED, RESET };
export const sc = (color: string, txt: string) => color + txt + RESET;
