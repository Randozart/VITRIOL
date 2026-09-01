// Braille gradient gauges — ported from VITRIOL vitriol-tui/src/braille.rs
// (Apache-2.0, our engine) + the Vitriolum color ramps from theme.rs, so the
// cockpit and the coding TUI speak the same visual language (2026-08-31).
// Ramp/muted colors now come from _shared/vitriolum.ts (single palette
// source for all extensions; parity-tested against theme.rs).
import { fgSeq, hexToRgb, VITRIOLUM, type VitriolumName } from "../_shared/vitriolum.ts";
import { getNative } from "../_shared/native.ts";
//
// Six-dot braille cells (U+2800..U+28FF), 6 percentage points per cell,
// filled bottom-left dot first, rising. Each lit cell is colored along a
// multi-stop ramp by the cell's position fraction across the bar; empty
// cells use the muted color.

export const FILL_ORDER = [0x04, 0x20, 0x02, 0x10, 0x01, 0x08];

export function glyph(mask: number): string {
  return String.fromCodePoint(0x2800 + mask);
}

export interface RGB {
  r: number;
  g: number;
  b: number;
}

export interface RampStop {
  at: number;
  color: RGB;
}

function hex(hexCode: number): RGB {
  return { r: (hexCode >> 16) & 0xff, g: (hexCode >> 8) & 0xff, b: hexCode & 0xff };
}

function lerp(a: RGB, b: RGB, t: number): RGB {
  return {
    r: Math.round(a.r + (b.r - a.r) * t),
    g: Math.round(a.g + (b.g - a.g) * t),
    b: Math.round(a.b + (b.b - a.b) * t),
  };
}

// Piecewise-lerped multi-stop ramp (Vitriolum ramps, VITRIOL theme.rs).
export class Ramp {
  private stops: RampStop[];
  /** Name for the native (Rust) renderer; matches officina/native/src/braille.rs. */
  name?: string;

  constructor(stops: RampStop[], name?: string) {
    this.stops = [...stops].sort((a, b) => a.at - b.at);
    this.name = name;
  }

  static fromHex(hexes: Array<[number, number]>): Ramp {
    return new Ramp(hexes.map(([at, c]) => ({ at, color: hex(c) })));
  }

  color(t: number): RGB {
    const x = Math.min(1, Math.max(0, t));
    if (x <= this.stops[0].at) return this.stops[0].color;
    const last = this.stops[this.stops.length - 1];
    if (x >= last.at) return last.color;
    for (let i = 0; i < this.stops.length - 1; i++) {
      const a = this.stops[i];
      const b = this.stops[i + 1];
      if (x >= a.at && x <= b.at) {
        const span = b.at - a.at || 1;
        return lerp(a.color, b.color, (x - a.at) / span);
      }
    }
    return last.color;
  }
}

// Named Vitriolum ramps (VITRIOL vitriol-tui/src/theme.rs), colors from the
// shared palette.
const palStop = (at: number, name: VitriolumName) => ({ at, color: hexToRgb(VITRIOLUM[name]) });
const WHITE = { r: 0xff, g: 0xff, b: 0xff }; // capacity ramp start (theme.rs literal)
export const RAMPS = {
  // capacity: white -> light yellow -> orange -> red -> deep red
  capacity: new Ramp([
    { at: 0, color: WHITE },
    palStop(0.25, "lightYellow"),
    palStop(0.5, "antidote"),
    palStop(0.75, "substrate"),
    palStop(1, "deepRed"),
  ], "capacity"),
  // activity: dark teal -> safety green -> solvent cyan
  activity: new Ramp([palStop(0, "darkTeal"), palStop(0.5, "safety"), palStop(1, "solvent")], "activity"),
  // mercury: muted gray -> solvent cyan (idle -> alive)
  mercury: new Ramp([palStop(0, "mercury"), palStop(1, "solvent")], "mercury"),
} as const;

export interface BarCell {
  mask: number;
  t: number;
}

// Prefix OR of FILL_ORDER: PREFIX_MASKS[k] = dots 0..k-1 lit (VITRIOL glyphs).
export const PREFIX_MASKS = [0x00, 0x04, 0x24, 0x26, 0x36, 0x37, 0x3f];

// Braille bar cells for ratio in [0,1]; `cells` braille columns.
export function barCells(ratio: number, cells: number): BarCell[] {
  const r = Math.min(1, Math.max(0, ratio));
  const filled = Math.round(r * cells * 6);
  const out: BarCell[] = [];
  for (let i = 0; i < cells; i++) {
    const dots = Math.min(6, Math.max(0, filled - i * 6));
    out.push({ mask: PREFIX_MASKS[dots], t: cells > 1 ? i / (cells - 1) : 0 });
  }
  return out;
}

function ansiFg(c: RGB): string {
  return `\x1b[38;2;${c.r};${c.g};${c.b}m`;
}

const ANSI_RESET = "\x1b[0m";

// Render a colored braille gauge: lit cells ramp-colored, empty cells muted.
// Delegates to the Rust addon (officina/native) when built — same output,
// no per-cell JS allocation. JS path kept as fallback.
export function renderGauge(ramp: Ramp, ratio: number, cells: number): string {
  const muted = fgSeq("gray");
  const n = getNative();
  if (n && ramp.name) {
    try {
      const { r, g, b } = hexToRgb(VITRIOLUM.gray);
      return n.renderGauge(ramp.name, ratio, cells, r, g, b);
    } catch {
      // fall through to the JS renderer
    }
  }
  let out = "";
  for (const cell of barCells(ratio, cells)) {
    if (cell.mask === 0) {
      out += muted + glyph(0);
    } else {
      out += ansiFg(ramp.color(cell.t)) + glyph(cell.mask);
    }
  }
  return out + ANSI_RESET;
}
