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
// Green streak (owner request 2026-09-03 — "nice and mysterious"): a
// narrower emerald band trailing the silver sweep through the stone.
const SHIMMER_STREAK_BAND: f64 = 3.0; // half the silver band — a streak
const SHIMMER_STREAK_OFFSET: f64 = 10.0; // cells behind the silver crest
const SHIMMER_STREAK_PEAK: (u8, u8, u8) = (0x36, 0x68, 0x46); // dim emerald
const BREATHE_PERIOD_MS: f64 = 5000.0;
const BREATHE_PEAK: (u8, u8, u8) = (0x35, 0x48, 0x5F);
const TWINKLE_STEP_MS: u128 = 240; // glint slot length
const TWINKLE_ODDS: u32 = 6; // ~N/1024 cells glint per slot
const TWINKLE_PEAK: (u8, u8, u8) = (0x55, 0x6B, 0x8C);

fn lerp8(base: u8, peak: u8, t: f64) -> u8 {
    (base as f64 + (peak as f64 - base as f64) * t).round() as u8
}

// ── Shimmer geometry (pure, shared with tests) ────────────────────────────

/// Sweep crest position for cycle fraction `p` in [0,1] over stone depth
/// `max_d`. Travel budget includes the streak offset so BOTH bands exit
/// the stone before the rest phase (owner bug 2026-09-03: the streak's
/// budget was never extended — it parked at d_max−1 mid-stone, and since
/// only the bottom rows reach those diagonals it glowed as a horizontal
/// dash through the whole stillness window).
fn sweep_pos(p: f64, max_d: f64) -> f64 {
    p * (max_d + SHIMMER_BAND * 2.0 + SHIMMER_STREAK_OFFSET) - SHIMMER_BAND
}

/// The emerald streak trails the silver crest at a fixed gap.
fn streak_pos(sweep: f64) -> f64 {
    sweep - SHIMMER_STREAK_OFFSET
}

/// Band intensity at diagonal `d` from crest `c` — linear falloff over
/// `band`, squared for a soft shoulder.
fn band_i(d: f64, c: f64, band: f64) -> f64 {
    let i = (1.0 - (d - c).abs() / band).clamp(0.0, 1.0);
    i * i
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
    let mut lines: Vec<String> = raw
        .lines()
        .map(|l| l.trim_end().to_string())
        .collect();
    strip_braille_margins(&mut lines);
    lines
}

/// Remove the invisible braille-blank (U+2800) margins hugging the art
/// (owner request 2026-09-03 — "a lot of whitespace before and after").
/// `trim_end()` above never sees U+2800: it is not ASCII whitespace, so
/// each row carries ~12 leading and ~12 trailing blank columns that
/// widen the rendered block far beyond the visible stone.
///
/// Leading: remove only the SHARED minimum blank run across all rows —
/// uniform column removal shifts every row together, so relative offsets
/// (the motto's indent against the stone) are mathematically preserved.
/// Trailing: per-line blank-run trim (left-alignment is unaffected).
/// Rows are never dropped: the diamond taper IS the logo (measured: every
/// one of the 32/38 rows carries dots).
fn strip_braille_margins(lines: &mut [String]) {
    let is_blank = |ch: char| ch == '⠀' || ch == ' ';
    let mut min_lead = usize::MAX;
    for l in lines.iter() {
        if l.chars().all(is_blank) {
            continue; // fully blank row imposes no margin
        }
        let lead = l.chars().take_while(|c| is_blank(*c)).count();
        min_lead = min_lead.min(lead);
    }
    if min_lead == usize::MAX {
        return; // all blank — nothing to strip
    }
    // (min_lead == 0 → start 0 for every row; the loop still trims trailing.)
    for l in lines.iter_mut() {
        let chars: Vec<char> = l.chars().collect();
        let start = min_lead.min(chars.len()).min(
            // never eat into a non-blank row's content beyond its own lead
            chars.iter().take_while(|c| is_blank(**c)).count(),
        );
        let mut end = chars.len();
        while end > start && is_blank(chars[end - 1]) {
            end -= 1;
        }
        *l = chars[start..end].iter().collect();
    }
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
            let pos = sweep_pos(p, max_d);
            // The emerald streak rides the same phase, trailing the silver
            // crest (owner request 2026-09-03).
            let pos_g = streak_pos(pos);
            lines
                .iter()
                .enumerate()
                .map(|(row, l)| {
                    let mut spans: Vec<Span> = Vec::new();
                    let mut plain = String::new();
                    // Merge key: (intensity, green?) — same color AND
                    // similar intensity coalesce into one span.
                    let mut cur: Option<(f64, bool)> = None;
                    for (col, ch) in l.chars().enumerate() {
                        let d = (col + row * 2) as f64;
                        let i_s = band_i(d, pos, SHIMMER_BAND);
                        let i_g = band_i(d, pos_g, SHIMMER_STREAK_BAND);
                        // Green wins where it's clearly present and at
                        // least as bright as the silver — the streak reads
                        // as its own light, not a tint on the sweep.
                        let green = i_g > 0.05 && i_g >= i_s;
                        let i = if green { i_g } else { i_s };
                        match cur {
                            Some((c, g)) if g == green && (i - c).abs() < 0.04 => plain.push(ch),
                            Some((c, g)) => {
                                spans.push(glint_span2(&plain, c, g));
                                plain = ch.to_string();
                                cur = Some((i, green));
                            }
                            None => {
                                plain.push(ch);
                                cur = Some((i, green));
                            }
                        }
                    }
                    if !plain.is_empty() {
                        if let Some((c, g)) = cur {
                            spans.push(glint_span2(&plain, c, g));
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
/// `green` selects the emerald streak peak (owner request 2026-09-03).
fn glint_span2(text: &str, i: f64, green: bool) -> Span<'static> {
    if i <= 0.02 {
        Span::styled(text.to_string(), Style::default().fg(theme::WATERMARK).add_modifier(Modifier::DIM))
    } else {
        let peak = if green { SHIMMER_STREAK_PEAK } else { SHIMMER_PEAK };
        Span::styled(text.to_string(), Style::default().fg(glint_color(peak, i)))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The emerald streak must stay MYSTERIOUS, not Christmas: at any
    /// intensity its peak luminance stays at or below the silver sweep's.
    #[test]
    fn streak_peak_luminance_bounded_by_silver() {
        let lum = |c: ratatui::style::Color| match c {
            ratatui::style::Color::Rgb(r, g, b) => 0.2126 * r as f64 + 0.7152 * g as f64 + 0.0722 * b as f64,
            _ => 0.0,
        };
        for i in [0.2f64, 0.5, 0.8, 1.0] {
            assert!(lum(glint_color(SHIMMER_STREAK_PEAK, i)) <= lum(glint_color(SHIMMER_PEAK, i)) + 1e-6);
        }
    }

    /// Green and silver spans at the same intensity render different
    /// colors — the streak is its own light, not a tint of the sweep.
    #[test]
    fn streak_color_differs_from_silver() {
        let s = glint_span2("a", 0.8, false);
        let g = glint_span2("a", 0.8, true);
        assert_ne!(s.style.fg, g.style.fg);
        // And below the merge threshold both fall back to the base style.
        let base_s = glint_span2("a", 0.01, false);
        let base_g = glint_span2("a", 0.01, true);
        assert_eq!(base_s.style.fg, base_g.style.fg);
    }

    /// Owner bug 2026-09-03 ("horizontal streak"): the streak's travel was
    /// never extended for its offset, so at rest it parked at the stone's
    /// deepest diagonals — which only the bottom rows reach, reading as a
    /// horizontal dash through the whole stillness window. Both bands must
    /// be fully off the stone during rest, and genuinely sweep it mid-cycle.
    #[test]
    fn shimmer_and_streak_fully_exit_before_rest() {
        let (width, show) = (80.0f64, 38.0f64);
        let max_d = width + show * 2.0; // travel basis used by render
        let d_max = max_d - 3.0; // deepest live cell (corner col/row)
        // The PARKED position (p clamps to 1.0 for the whole rest phase —
        // 55% of the cycle) must be fully off-stone for both bands.
        // (p < 1.0 is mid-sweep: bands legitimately transit the stone.)
        let pos = sweep_pos(1.0, max_d);
        assert_eq!(band_i(d_max, pos, SHIMMER_BAND), 0.0, "silver parked");
        assert_eq!(
            band_i(d_max, streak_pos(pos), SHIMMER_STREAK_BAND),
            0.0,
            "streak parked"
        );
        // Mid-sweep the crest truly crosses the stone: at the cycle fraction
        // that places the crest mid-stone, the center cell burns at full.
        let mid_p = (max_d / 2.0 + SHIMMER_BAND)
            / (max_d + SHIMMER_BAND * 2.0 + SHIMMER_STREAK_OFFSET);
        assert!(band_i(max_d / 2.0, sweep_pos(mid_p, max_d), SHIMMER_BAND) > 0.99);
    }

    /// Owner request 2026-09-03: strip the invisible braille-blank margins.
    /// Shared leading columns go; per-row EXTRA leading (the motto's indent)
    /// must survive — relative offsets are the whole point.
    #[test]
    fn strip_removes_shared_margins_but_keeps_relative_offsets() {
        let mut lines = vec![
            "⠀⠀⠀⠀⢀⣠⡀⠀".to_string(),   // lead 4, content, trailing blank
            "⠀⠀⠀⠀⢺⣟⣻⢿⣿".to_string(), // lead 4
            "⠀⠀⠀⠀⠀⠀⣼⣽".to_string(),   // lead 6 — the "motto" indent
        ];
        strip_braille_margins(&mut lines);
        assert_eq!(lines[0], "⢀⣠⡀");
        assert_eq!(lines[1], "⢺⣟⣻⢿⣿");
        assert_eq!(lines[2], "⠀⠀⣼⣽", "extra indent beyond the shared run survives");
    }

    #[test]
    fn strip_handles_blank_and_empty_lines() {
        let mut lines = vec![String::new(), "⠀⠀⠀⠀a b".to_string(), "⠀".to_string()];
        strip_braille_margins(&mut lines);
        assert_eq!(lines[0], "");
        assert_eq!(lines[1], "a b");
        assert_eq!(lines[2], "");
    }

    /// The shipped assets: strip tightens the block to the visible art and
    /// the motto rows keep their indent relative to the stone rows.
    /// (Note: stone rows lead with U+2800, motto rows with ASCII spaces —
    /// the shared-lead minimum must count BOTH blank kinds, matching the
    /// strip's own `is_blank`.)
    #[test]
    fn shipped_assets_strip_to_visible_bbox() {
        let is_blank = |ch: char| ch == '⠀' || ch == ' ';
        let lead = |l: &str| l.chars().take_while(|c| is_blank(*c)).count();
        let plain = art_lines(LOGO_RAW);
        let w = plain.iter().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(w < 80, "plain block should tighten below the raw 80, got {}", w);
        assert_eq!(plain.len(), 32, "no rows dropped — the taper IS the logo");
        let motto = art_lines(LOGO_MOTTO_RAW);
        assert_eq!(motto.len(), 38, "no rows dropped from the motto version");
        // First stone row and first motto row: the motto's offset relative
        // to the stone is preserved exactly as in the raw asset.
        let raw: Vec<&str> = LOGO_MOTTO_RAW.lines().collect();
        let delta_raw = lead(raw[32]) as isize - lead(raw[0]) as isize;
        let delta_out = lead(&motto[32]) as isize - lead(&motto[0]) as isize;
        assert_eq!(delta_raw, delta_out, "motto offset relative to stone unchanged");
    }
}
