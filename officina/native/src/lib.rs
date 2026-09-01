//! officina-native — NAPI addon exposing Officina hot paths to the
//! TypeScript extensions and the patched vendor layout.
//!
//! Load path: .pi/extensions/_shared/native.ts (createRequire on index.node,
//! publishing globalThis.__officinaNative so runtime/patched JS can delegate
//! without touching Node resolution). Every caller keeps a JS fallback — a
//! missing or stale addon costs performance, never correctness.
//!
//! Provenance: original work, this repo (Apache-2.0 OR MIT).

#![deny(clippy::all)]

mod ansi;
mod braille;

use napi_derive::napi;

/// Strip CSI/OSC/APC sequences. (JS fallback: officinaStripZeroWidth)
#[napi]
pub fn strip_ansi(line: String) -> String {
    ansi::strip_ansi(&line)
}

/// Visible terminal width, escape-aware, wide-char aware.
/// (JS fallback: officinaVisibleWidth)
#[napi]
pub fn visible_width(line: String) -> u32 {
    ansi::visible_width(&line)
}

/// Truncate to `width` visible cells preserving escape sequences.
/// (JS fallback: officinaCut)
#[napi]
pub fn cut_line(line: String, width: u32) -> String {
    ansi::cut(&line, width)
}

/// Whole OfficinaSplit row-merge loop in one native call — the per-render
/// hot path. (JS fallback: the merge loop inside OfficinaSplit.render)
#[napi(object)]
pub struct MergeInput {
    pub main_lines: Vec<String>,
    pub main_w: u32,
    pub sb_lines: Vec<String>,
    pub sb_w: u32,
    /// P4 bottom-anchor: rows of empty sidebar before sb_lines.
    pub sb_pad: u32,
    pub gap: u32,
    pub bg: String,
    pub reset: String,
}

#[napi]
pub fn merge_split_rows(input: MergeInput) -> Vec<String> {
    ansi::merge_split_rows(
        &input.main_lines,
        input.main_w,
        &input.sb_lines,
        input.sb_w,
        input.sb_pad,
        input.gap,
        &input.bg,
        &input.reset,
    )
}

/// Render a Vitriolum braille gauge. `mutedR/G/B` is the empty-cell color.
/// (JS fallback: vitriol-decode/braille.ts renderGauge)
#[napi]
pub fn render_gauge(
    ramp: String,
    ratio: f64,
    cells: u32,
    muted_r: i32,
    muted_g: i32,
    muted_b: i32,
) -> napi::Result<String> {
    let muted = (
        muted_r.clamp(0, 255) as u8,
        muted_g.clamp(0, 255) as u8,
        muted_b.clamp(0, 255) as u8,
    );
    braille::render_gauge(&ramp, ratio, cells, muted).map_err(napi::Error::from_reason)
}

/// Flattened ramp stops ([at×1000, r, g, b] per stop) — parity-test hook so
/// vitest can assert the Rust palette matches _shared/vitriolum.ts.
#[napi]
pub fn ramp_stops(ramp: String) -> napi::Result<Vec<i32>> {
    braille::ramp_stops(&ramp).map_err(napi::Error::from_reason)
}

/// Addon version — lets the TS loader verify it loaded the build it expects.
#[napi]
pub fn native_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
