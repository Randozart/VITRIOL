// Animated single-line status for Plan Mode's reasoning phases ("deciding what
// to explore…", "preparing clarifying questions…", etc.). Shows a spinner that
// animates and a running m:ss timer counting total plan-mode time, so the user
// can see it's working and how long it's taken.
//
// Same approach as the sub-coder tracker: string[] re-set on a ~120ms timer
// (needed to animate the spinner + tick the clock), colored with raw SGR so it
// doesn't depend on the active theme.

import { terminalColumns, truncateLineToWidth } from "../_shared/width.ts";

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const honey = (s: string) => `\x1b[38;2;225;90;31m${s}\x1b[39m`;
const gray = (s: string) => `\x1b[90m${s}\x1b[39m`;

function fmtElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(total / 60)}:${(total % 60).toString().padStart(2, "0")}`;
}

export interface StatusUI {
  hasUI: boolean;
  ui: {
    setWidget: (
      key: string,
      content: string[] | undefined,
      options?: { placement?: "aboveEditor" | "belowEditor" },
    ) => void;
  };
}

export class PlanStatus {
  private message = "";
  private startMs = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private lastFrame = "";

  constructor(
    private ctx: StatusUI,
    private key = "plan-status",
    private placement: "aboveEditor" | "belowEditor" = "aboveEditor",
  ) {}

  /**
   * Begin showing `message` and start animating. Pass `since` (a timestamp) to
   * keep one continuous timer across phases — the elapsed shown counts from
   * `since`, not from when start() was called.
   */
  start(message: string, since?: number): void {
    if (!this.ctx.hasUI) return;
    this.message = message;
    this.startMs = since ?? Date.now();
    this.render();
    if (!this.timer) this.timer = setInterval(() => this.render(), 120);
  }

  /** Switch the phase message; the timer keeps counting total elapsed. */
  set(message: string): void {
    this.message = message;
    if (this.ctx.hasUI) this.render();
  }

  /** Stop the animation and clear the line. */
  stop(): void {
    if (this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    }
    if (this.ctx.hasUI) this.ctx.ui.setWidget(this.key, undefined, { placement: this.placement });
  }

  private render(): void {
    if (!this.ctx.hasUI) return;
    const now = Date.now();
    const frame = SPINNER[Math.floor(now / 100) % SPINNER.length];
    const raw = `${honey(frame)} ${this.message}  ${gray(fmtElapsed(now - this.startMs))}`;
    // Cap to terminal width — pi-tui throws on overflow (issue #48). Our own
    // phase messages are short, but the line is still passed through for
    // defense-in-depth (a future caller, or a long custom message, won't crash).
    const line = truncateLineToWidth(raw, terminalColumns());
    if (line === this.lastFrame) return; // diff-guard
    this.lastFrame = line;
    this.ctx.ui.setWidget(this.key, [line], { placement: this.placement });
  }
}
