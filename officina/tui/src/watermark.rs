//! Watermark — the VITRIOL braille logo, faint, on fresh screens.
//!
//! Ported from vitriol-tui/src/watermark.rs (this repo, Apache-2.0) with
//! three local changes: the art is embedded at compile time via include_str!
//! so the binary works from any working directory; the block sits in the
//! precise middle of the space it occupies (both axes) on a fresh screen
//! (zero entries) in a barely-there blue lift (#2a3a52 on #0d1117); and the
//! stone glimmers (2026-09-02, owner request — "Christmas lights"
//! configurability). Art source: the stone-with-motto
//! (braille-logo-80c-motto.txt) whenever the window height allows it, the
//! plain stone (braille-logo-80c.txt) otherwise. Missing asset or
//! too-small area = silently nothing — a cut braille stone looks broken.

use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;

const LOGO_RAW: &str = include_str!("../../../assets/braille-logo-80c.txt");
const LOGO_MOTTO_RAW: &str = include_str!("../../../assets/braille-logo-80c-motto.txt");

// ── Glimmer engine (owner request 2026-09-02) ────────────────────────────
// Three animation modes + off, cycled by the /glimmer local command and
// persisted by the run loop. All modes derive everything from `phase_ms` —
// no internal state, so any frame is reproducible from the clock.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlimmerMode {
    /// Narrow diagonal band sweeping the stone (~8 s per sweep).
    Shimmer,
    /// Whole-stone slow pulse (~5 s cycle).
    Breathe,
    /// Sparse cells glint briefly, like stars.
    Twinkle,
    /// Static stone.
    Off,
}

impl GlimmerMode {
    pub const ALL: [GlimmerMode; 4] = [
        GlimmerMode::Shimmer,
        GlimmerMode::Breathe,
        GlimmerMode::Twinkle,
        GlimmerMode::Off,
    ];

    /// Name for the notify line / persistence file.
    pub fn label(self) -> &'static str {
        match self {
            GlimmerMode::Shimmer => "shimmer",
            GlimmerMode::Breathe => "breathe",
            GlimmerMode::Twinkle => "twinkle",
            GlimmerMode::Off => "off",
        }
    }

    pub fn parse(s: &str) -> Option<GlimmerMode> {
        GlimmerMode::ALL.iter().copied().find(|m| m.label() == s)
    }

    pub fn next(self) -> GlimmerMode {
        let i = GlimmerMode::ALL.iter().position(|m| *m == self).unwrap_or(0);
        GlimmerMode::ALL[(i + 1) % GlimmerMode::ALL.len()]
    }
}

// Tuning constants — one place to re-voice the effect.
const SHIMMER_PERIOD_MS: f64 = 6000.0;
const SHIMMER_SWEEP_FRAC: f64 = 0.45; // sweep vs stillness, per cycle
const SHIMMER_BAND: f64 = 6.0; // cells, diagonal width at half intensity
const SHIMMER_PEAK: (u8, u8, u8) = (0x4A, 0x5F, 0x7D);
const BREATHE_PERIOD_MS: f64 = 5000.0;
const BREATHE_PEAK: (u8, u8, u8) = (0x35, 0x48, 0x5F);
const TWINKLE_STEP_MS: u128 = 240; // glint slot length
const TWINKLE_ODDS: u32 = 6; // ~N/1024 cells glint per slot
const TWINKLE_PEAK: (u8, u8, u8) = (0x55, 0x6B, 0x8C);

fn lerp8(base: u8, peak: u8, t: f64) -> u8 {
    (base as f64 + (peak as f64 - base as f64) * t).round() as u8
}

/// Mix WATERMARK toward a peak tint by fraction `t` in [0, 1].
fn glint_color(peak: (u8, u8, u8), t: f64) -> ratatui::style::Color {
    let (base_r, base_g, base_b) = (0x2A, 0x3A, 0x52); // theme::WATERMARK
    ratatui::style::Color::Rgb(
        lerp8(base_r, peak.0, t),
        lerp8(base_g, peak.1, t),
        lerp8(base_b, peak.2, t),
    )
}

/// Deterministic 32-bit cell hash (no RNG state; any frame reproducible).
pub(crate) fn cell_hash(x: u32, y: u32, step: u32) -> u32 {
    let mut h = x
        .wrapping_mul(73856093)
        ^ y.wrapping_mul(19349663)
        ^ step.wrapping_mul(83492791);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846CA68B);
    h ^= h >> 16;
    h
}

/// Non-empty art lines from an embedded asset (trailing blank rows dropped).
fn art_lines(raw: &'static str) -> Vec<String> {
    raw.lines()
        .map(|l| l.trim_end().to_string())
        .collect::<Vec<_>>()
}

/// The watermark art: the stone-with-motto when the window can hold it, the
/// plain stone otherwise (owner request 2026-09-02). Both assets carry their
/// own internal centering; trailing blank rows are trimmed.
fn pick_art(height: usize) -> Vec<String> {
    let motto = art_lines(LOGO_MOTTO_RAW);
    if motto.len() <= height {
        motto
    } else {
        art_lines(LOGO_RAW)
    }
}

/// Draw the block, centered on both axes inside `area` — but only if the
/// area fits every row. No partial reveal. `phase_ms` drives the glimmer.
/// The stone-with-motto rides whenever the height allows it, the plain stone
/// otherwise (owner request 2026-09-02).
pub fn render(frame: &mut Frame, area: Rect, phase_ms: u128, mode: GlimmerMode) {
    if area.height < 4 || area.width < 20 {
        return;
    }
    let lines = pick_art(area.height as usize);
    if (area.height as usize) < lines.len() {
        return;
    }
    let show = lines.len();
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    // Precise middle of the space the block occupies (owner request
    // 2026-09-02): vertical center within the watermark area, horizontal
    // block-center (pad from the widest line, applied to every line).
    let left_pad = area.width.saturating_sub(width as u16) / 2;
    let y = area.y + (area.height - show as u16) / 2;
    let rect = Rect {
        x: area.x,
        y,
        width: area.width,
        height: show as u16,
    };

    let base = Style::default()
        .fg(theme::WATERMARK)
        .add_modifier(Modifier::DIM);

    let spans: Vec<Line> = match mode {
        GlimmerMode::Off | GlimmerMode::Breathe => {
            let style = if mode == GlimmerMode::Breathe {
                let t = 0.5
                    - 0.5 * (2.0 * std::f64::consts::PI * phase_ms as f64 / BREATHE_PERIOD_MS)
                        .cos();
                Style::default().fg(glint_color(BREATHE_PEAK, t))
            } else {
                base
            };
            lines
                .iter()
                .map(|l| Line::from(Span::styled(format!("{}{}", " ".repeat(left_pad as usize), l), style)))
                .collect()
        }
        GlimmerMode::Shimmer => {
            let t = phase_ms as f64 / SHIMMER_PERIOD_MS;
            let max_d = (width + show * 2) as f64;
            // The sweep LOOPS (owner bug report 2026-09-02: it played once
            // on open and never returned — `pos` ran past the stone and
            // stayed there forever). Each cycle: one sweep (~45% of the
            // period), then stillness while the phase wraps around.
            let cycle = t.fract();
            let p = (cycle / SHIMMER_SWEEP_FRAC).min(1.0);
            let pos = p * (max_d + SHIMMER_BAND * 2.0) - SHIMMER_BAND;
            lines
                .iter()
                .enumerate()
                .map(|(row, l)| {
                    let mut spans: Vec<Span> = Vec::new();
                    let mut plain = String::new();
                    let mut cur: Option<f64> = None;
                    for (col, ch) in l.chars().enumerate() {
                        let d = (col + row * 2) as f64;
                        let i = (1.0 - (d - pos).abs() / SHIMMER_BAND).clamp(0.0, 1.0);
                        let i = i * i; // soft falloff
                        match cur {
                            Some(c) if (i - c).abs() < 0.04 => plain.push(ch),
                            Some(c) => {
                                spans.push(glint_span(&plain, c));
                                plain = ch.to_string();
                                cur = Some(i);
                            }
                            None => {
                                plain.push(ch);
                                cur = Some(i);
                            }
                        }
                    }
                    if !plain.is_empty() {
                        if let Some(c) = cur {
                            spans.push(glint_span(&plain, c));
                        }
                    }
                    Line::from(
                        std::iter::once(Span::raw(" ".repeat(left_pad as usize)))
                            .chain(spans.into_iter())
                            .collect::<Vec<Span>>(),
                    )
                })
                .collect()
        }
        GlimmerMode::Twinkle => {
            let step = (phase_ms / TWINKLE_STEP_MS) as u32;
            lines
                .iter()
                .enumerate()
                .map(|(row, l)| {
                    let mut spans: Vec<Span> = Vec::new();
                    let mut plain = String::new();
                    let mut cur: Option<u32> = None;
                    for (col, ch) in l.chars().enumerate() {
                        let h = cell_hash(col as u32, row as u32, step);
                        let g = if h % 1024 < TWINKLE_ODDS { Some(h % 128) } else { None };
                        match (cur, g) {
                            (Some(c), Some(n)) if (c / 32) == (n / 32) => plain.push(ch),
                            (Some(c), _) => {
                                spans.push(twinkle_span(&plain, c));
                                plain = ch.to_string();
                                cur = g;
                            }
                            (None, Some(n)) => {
                                plain.push(ch);
                                cur = Some(n);
                            }
                            (None, None) => plain.push(ch),
                        }
                    }
                    if !plain.is_empty() {
                        if let Some(c) = cur {
                            spans.push(twinkle_span(&plain, c));
                        }
                    }
                    Line::from(
                        std::iter::once(Span::raw(" ".repeat(left_pad as usize)))
                            .chain(spans.into_iter())
                            .collect::<Vec<Span>>(),
                    )
                })
                .collect()
        }
    };
    frame.render_widget(Paragraph::new(spans).alignment(Alignment::Left), rect);
}

/// Span at shimmer intensity `i` in [0, 1] — base style below the merge
/// threshold, glint tint above (DIM dropped so the tint reads).
fn glint_span(text: &str, i: f64) -> Span<'static> {
    if i <= 0.02 {
        Span::styled(text.to_string(), Style::default().fg(theme::WATERMARK).add_modifier(Modifier::DIM))
    } else {
        Span::styled(text.to_string(), Style::default().fg(glint_color(SHIMMER_PEAK, i)))
    }
}

/// Span for a twinkle slot: `seed` (0..128) sets the glint brightness.
fn twinkle_span(text: &str, seed: u32) -> Span<'static> {
    let t = seed as f64 / 128.0 * 0.75; // cap below full peak for subtlety
    if t <= 0.02 {
        Span::styled(text.to_string(), Style::default().fg(theme::WATERMARK).add_modifier(Modifier::DIM))
    } else {
        Span::styled(text.to_string(), Style::default().fg(glint_color(TWINKLE_PEAK, t)))
    }
}
