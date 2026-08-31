// Width-aware truncation for custom TUI widget content.
//
// pi-tui (>= ~0.75) throws "Rendered line N exceeds terminal width" when any
// rendered line is wider than the terminal — see issue #48. Custom widgets
// (subagent tracker, plan-mode status/indicator, branding header) MUST cap
// every line they emit to the active terminal width, or pi crashes the whole
// session when the widget happens to produce a long line (e.g. a failed
// sub-coder whose errorMessage runs ~200 chars).
//
// pi-tui itself exports visibleWidth + truncateToWidth, but pi 0.79 no longer
// hoists pi-tui to the top-level node_modules, so extensions can't import it
// directly. This module is the lightweight inline replacement: it strips
// SGR / OSC escapes for the width count and walks the string char-by-char to
// truncate while preserving any in-flight color codes.

const SGR_RE = /\x1b\[[0-9;]*[a-zA-Z]/g;
const OSC_RE = /\x1b\][^\x07]*\x07/g;

export function stripAnsi(s: string): string {
  return s.replace(SGR_RE, "").replace(OSC_RE, "");
}

// Approximation: ANSI-stripped char count (treats every visible char as width
// 1). Exact for ASCII / Latin / single-cell glyphs the tracker and status
// widgets emit. Wide CJK or emoji *under*count here, so callers should leave a
// small safety margin (see terminalColumns/SAFETY_MARGIN).
export function visibleWidth(s: string): number {
  return [...stripAnsi(s)].length;
}

// Truncate `line` so its visible width fits `maxWidth`. ANSI escapes are
// preserved verbatim; the visible portion is cut at maxWidth-1 to leave room
// for an ellipsis, and a final SGR reset is appended so a half-emitted color
// can't bleed into the next line. If maxWidth ≤ 0, returns "" (defensive).
export function truncateLineToWidth(line: string, maxWidth: number, ellipsis = "…"): string {
  if (maxWidth <= 0) return "";
  if (visibleWidth(line) <= maxWidth) return line;
  const cutAt = Math.max(0, maxWidth - visibleWidth(ellipsis));
  let visible = 0;
  let out = "";
  let i = 0;
  while (i < line.length) {
    const ch = line[i];
    if (ch === "\x1b" && line[i + 1] === "[") {
      const m = line.slice(i).match(/^\x1b\[[0-9;]*[a-zA-Z]/);
      if (m) {
        out += m[0];
        i += m[0].length;
        continue;
      }
    }
    if (ch === "\x1b" && line[i + 1] === "]") {
      const end = line.indexOf("\x07", i);
      if (end >= 0) {
        out += line.slice(i, end + 1);
        i = end + 1;
        continue;
      }
    }
    if (visible >= cutAt) break;
    out += ch;
    visible += 1;
    i += 1;
  }
  return out + ellipsis + "\x1b[0m";
}

// Current terminal width, with a small safety margin to absorb wide Unicode
// chars that visibleWidth's char-count approximation under-measures. Falls
// back to `fallback` columns when stdout isn't a TTY (headless runs).
const SAFETY_MARGIN = 2;
export function terminalColumns(fallback = 80): number {
  const c = (process.stdout && (process.stdout as any).columns) | 0;
  const w = c > 0 ? c : fallback;
  return Math.max(20, w - SAFETY_MARGIN);
}
