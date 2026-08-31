// Rectangle progress bar for the deep-research flow.
//
// Replaces the multi-row sub-coder "subagent view" (SubCoderTracker) for THIS
// flow only: instead of one animated row per child, it shows a single filled
// rectangle and an "X/N agents" count, so a long serial run reads as steady
// forward progress rather than a wall of spinners.
//
// Same mechanics as plan-mode's PlanStatus: a string[] re-set on a ~120ms timer
// (to tick the clock), colored with raw SGR so it's theme-independent, and every
// emitted line capped to the terminal width — pi-tui throws and crashes the whole
// session on any overflowing widget line (issue #48).

import { terminalColumns, truncateLineToWidth } from "../_shared/width.ts";

const honey = (s: string) => `\x1b[38;2;225;90;31m${s}\x1b[39m`;
const gray = (s: string) => `\x1b[90m${s}\x1b[39m`;

const BAR_MAX = 40;
const BAR_MIN = 4;

function fmtElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, "0")}`;
}

export interface ProgressState {
  done: number;
  total: number;
  phase: string;
  elapsedMs: number;
  width: number;
}

// Pure renderer — exported so tests can assert fill math and width-safety without
// a live terminal. Returns 1-2 already-truncated lines ready for setWidget.
export function renderProgressLines(s: ProgressState): string[] {
  const total = Math.max(1, Math.floor(s.total));
  const done = Math.max(0, Math.min(Math.floor(s.done), total));
  const count = `${done}/${total} agents`;

  const header = truncateLineToWidth(
    `${honey("deep research")} ${gray("·")} ${s.phase}   ${gray(count)}   ${gray(fmtElapsed(s.elapsedMs))}`,
    s.width,
  );

  // Bar width tracks the terminal but is bounded so it stays a tidy rectangle and
  // never risks overflow (kept a couple columns inside terminalColumns).
  const barWidth = Math.max(BAR_MIN, Math.min(BAR_MAX, s.width - 2));
  const filled = Math.max(0, Math.min(barWidth, Math.round((barWidth * done) / total)));
  const bar = honey("▓".repeat(filled)) + gray("░".repeat(barWidth - filled));

  return [header, truncateLineToWidth(bar, s.width)];
}

export interface ProgressUI {
  hasUI: boolean;
  ui: {
    setWidget: (
      key: string,
      content: string[] | undefined,
      options?: { placement?: "aboveEditor" | "belowEditor" },
    ) => void;
  };
}

export class ResearchProgress {
  private done = 0;
  private total = 1;
  private phase = "";
  private startMs = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private lastFrame = "";

  constructor(
    private ctx: ProgressUI,
    private key = "deep-research-progress",
    private placement: "aboveEditor" | "belowEditor" = "aboveEditor",
  ) {}

  /** Start the bar. `since` keeps one continuous timer across phases. */
  begin(total: number, phase: string, since?: number): void {
    this.total = Math.max(1, total);
    this.done = 0;
    this.phase = phase;
    this.startMs = since ?? Date.now();
    if (!this.ctx.hasUI) return;
    this.render();
    if (!this.timer) this.timer = setInterval(() => this.render(), 120);
  }

  /** Update the completed count, and optionally the total and/or phase label. */
  update(done: number, total?: number, phase?: string): void {
    this.done = done;
    if (typeof total === "number") this.total = Math.max(1, total);
    if (phase) this.phase = phase;
    if (this.ctx.hasUI) this.render();
  }

  /** Switch the phase label; the timer and counts keep going. */
  setPhase(phase: string): void {
    this.phase = phase;
    if (this.ctx.hasUI) this.render();
  }

  /** Stop the animation and clear the widget. */
  end(): void {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
    if (this.ctx.hasUI) this.ctx.ui.setWidget(this.key, undefined, { placement: this.placement });
  }

  private render(): void {
    if (!this.ctx.hasUI) return;
    const lines = renderProgressLines({
      done: this.done,
      total: this.total,
      phase: this.phase,
      elapsedMs: Date.now() - this.startMs,
      width: terminalColumns(),
    });
    const joined = lines.join("\n");
    if (joined === this.lastFrame) return; // diff-guard
    this.lastFrame = joined;
    this.ctx.ui.setWidget(this.key, lines, { placement: this.placement });
  }
}
