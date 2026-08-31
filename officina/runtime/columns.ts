// Columns — horizontal layout for the Officina fork (2026-08-31, step 1 of
// docs/LAYOUT-FORK-2026-08-31.md). pi-tui's Box/Container stack vertically
// only; the docked sidebar needs side-by-side columns. Provenance: original
// work, this repo, against the pinned pi 0.83.0 Component contract
// (render(width): string[], invalidate(), optional handleInput).
//
// Self-contained on purpose: ANSI visible-width + safe cutting live here so
// the fork adds exactly one new file to the layout layer (no pi-tui deep
// imports, which are not addressable through the package's exports map).
// Wide chars are counted as width 2 (good enough for ASCII art + CJK-ish).

export interface ComponentLike {
  render(width: number): string[];
  invalidate?(): void;
  handleInput?(data: string): void;
}

export interface ColumnSpec {
  component: ComponentLike;
  /** Fixed width in cells, or a share of remaining width (e.g. { share: 0.3 }). */
  width: number | { share: number };
}

const ANSI_RX = /\x1b\[[0-9;?]*[a-zA-Z]/g;

export function visibleWidth(line: string): number {
  const bare = line.replace(ANSI_RX, "");
  let w = 0;
  for (const ch of bare) {
    const cp = ch.codePointAt(0) ?? 0;
    w += cp >= 0x1100 && (cp <= 0x115f || cp === 0x2329 || cp === 0x232a || (cp >= 0x2e80 && cp <= 0xa4cf) || (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xf900 && cp <= 0xfaff) || (cp >= 0xfe30 && cp <= 0xfe6f) || (cp >= 0xff00 && cp <= 0xff60) || (cp >= 0xffe0 && cp <= 0xffe6) || (cp >= 0x1f300 && cp <= 0x1f64f)) ? 2 : 1;
  }
  return w;
}

/** Cut a possibly-ANSI line to `width` visible cells, preserving escapes. */
export function cutAnsi(line: string, width: number): string {
  let vis = 0;
  let out = "";
  let i = 0;
  while (i < line.length) {
    if (line[i] === "\x1b") {
      const rest = line.slice(i);
      const m = /^(\x1b\[[0-9;?]*[a-zA-Z])/.exec(rest);
      if (m) {
        out += m[1];
        i += m[1].length;
        continue;
      }
    }
    if (vis >= width) {
      i++;
      continue;
    }
    const cp = line.codePointAt(i) ?? 0;
    const ch = String.fromCodePoint(cp);
    const cw = cp >= 0x1100 && (cp <= 0x115f || (cp >= 0x2e80 && cp <= 0xa4cf) || (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xff00 && cp <= 0xff60)) ? 2 : 1;
    if (vis + cw > width) {
      i += ch.length;
      continue;
    }
    out += ch;
    vis += cw;
    i += ch.length;
  }
  return out + "\x1b[0m";
}

export class Columns {
  children: ColumnSpec[];
  gap: number;

  constructor(children: ColumnSpec[] = [], gap = 1) {
    this.children = children;
    this.gap = gap;
  }

  addChild(spec: ColumnSpec): void {
    this.children.push(spec);
  }

  clear(): void {
    this.children = [];
  }

  private slotWidths(width: number): number[] {
    const total = Math.max(0, width - this.gap * Math.max(0, this.children.length - 1));
    const out: number[] = [];
    let flexTotal = 0;
    let used = 0;
    for (const c of this.children) {
      if (typeof c.width === "number") {
        out.push(c.width);
        used += c.width;
      } else {
        flexTotal += c.width.share;
        out.push(-1);
      }
    }
    const flexPx = Math.max(0, total - used);
    for (let i = 0; i < out.length; i++) {
      if (out[i] === -1) {
        const spec = this.children[i];
        const share = (spec.width as { share: number }).share;
        out[i] = flexTotal > 0 ? Math.floor((flexPx * share) / flexTotal) : 0;
      }
    }
    return out;
  }

  render(width: number): string[] {
    const slots = this.slotWidths(width);
    const rendered = this.children.map((c, i) => c.component.render(slots[i] ?? 0));
    const height = rendered.reduce((m, ls) => Math.max(m, ls.length), 0);
    const pad = " ".repeat(this.gap);
    const lines: string[] = [];
    for (let row = 0; row < height; row++) {
      let line = "";
      for (let i = 0; i < this.children.length; i++) {
        const slot = slots[i] ?? 0;
        let cell = rendered[i][row] ?? "";
        if (visibleWidth(cell) > slot) cell = cutAnsi(cell, slot);
        line += cell + " ".repeat(Math.max(0, slot - visibleWidth(cell)));
        if (i < this.children.length - 1) line += pad;
      }
      lines.push(line);
    }
    return lines;
  }
}
