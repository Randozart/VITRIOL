//! Vitriolum braille gauge rendering — Rust port of
//! officina/.pi/extensions/vitriol-decode/braille.ts (renderGauge), which
//! itself ports vitriol-tui/src/braille.rs. Six-dot cells (U+2800..U+28FF),
//! prefix-filled masks, piecewise-lerped multi-stop ramps.
//!
//! Provenance: original work, this repo (Apache-2.0 OR MIT) — port of our own
//! code within the same workspace. Palette constants mirror _shared/vitriolum.ts
//! and vitriol-tui/src/theme.rs; `ramp_stops` is exported so the vitest
//! parity test can assert native/vitestriolum agreement.

pub type Rgb = (u8, u8, u8);

/// PREFIX_MASKS[k] = bottom-left-first prefix of k lit dots (VITRIOL glyphs).
pub const PREFIX_MASKS: [u32; 7] = [0x00, 0x04, 0x24, 0x26, 0x36, 0x37, 0x3f];

struct Stop {
    at: f64,
    color: Rgb,
}

struct Ramp {
    stops: Vec<Stop>,
    name: &'static str,
}

fn ramp(name: &'static str, stops: &[(f64, Rgb)]) -> Ramp {
    let mut v: Vec<Stop> = stops.iter().map(|&(at, color)| Stop { at, color }).collect();
    v.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap());
    Ramp { stops: v, name }
}

/// Vitriolum ramps (vitriol-tui/src/theme.rs / _shared/vitriolum.ts).
fn find_ramp(name: &str) -> Option<&'static Ramp> {
    use std::sync::OnceLock;
    static RAMPS: OnceLock<[Ramp; 3]> = OnceLock::new();
    RAMPS.get_or_init(|| {
        [
            ramp(
                "capacity",
                &[
                    (0.0, (0xff, 0xff, 0xff)),
                    (0.25, (0xff, 0xe0, 0x66)), // lightYellow
                    (0.5, (0xff, 0x5f, 0x1f)),  // antidote
                    (0.75, (0xf8, 0x51, 0x49)), // substrate
                    (1.0, (0x8a, 0x15, 0x15)),  // deepRed
                ],
            ),
            ramp(
                "activity",
                &[
                    (0.0, (0x0b, 0x5e, 0x4c)), // darkTeal
                    (0.5, (0x3f, 0xb9, 0x50)), // safety
                    (1.0, (0x00, 0xff, 0xff)), // solvent
                ],
            ),
            ramp(
                "mercury",
                &[
                    (0.0, (0x55, 0x60, 0x6e)), // mercury
                    (1.0, (0x00, 0xff, 0xff)), // solvent
                ],
            ),
        ]
    })
    .iter()
    .find(|r| r.name == name)
}

fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn ramp_color(r: &Ramp, t: f64) -> Rgb {
    let x = t.clamp(0.0, 1.0);
    let first = &r.stops[0];
    let last = &r.stops[r.stops.len() - 1];
    if x <= first.at {
        return first.color;
    }
    if x >= last.at {
        return last.color;
    }
    for pair in r.stops.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if x >= a.at && x <= b.at {
            let span = (b.at - a.at).max(1.0);
            let _ = span; // keep parity with JS: span = b.at - a.at || 1
            let real_span = b.at - a.at;
            let denom = if real_span == 0.0 { 1.0 } else { real_span };
            return lerp(a.color, b.color, (x - a.at) / denom);
        }
    }
    last.color
}

fn glyph(mask: u32) -> char {
    char::from_u32(0x2800 + mask).unwrap_or('\u{2800}')
}

fn ansi_fg(c: Rgb) -> String {
    format!("\x1b[38;2;{};{};{}m", c.0, c.1, c.2)
}

/// Render a colored braille gauge: lit cells ramp-colored by position
/// fraction, empty cells in `muted`, SGR reset appended. Identical output to
/// the TS renderGauge.
pub fn render_gauge(ramp_name: &str, ratio: f64, cells: u32, muted: Rgb) -> Result<String, String> {
    if cells == 0 {
        return Err("cells must be > 0".to_string());
    }
    let r = find_ramp(ramp_name).ok_or_else(|| format!("unknown ramp: {ramp_name}"))?;
    let ratio = ratio.clamp(0.0, 1.0);
    // JS Math.round: half away from zero — same as Rust f64::round.
    let filled = (ratio * cells as f64 * 6.0).round() as i64;
    let mut out = String::with_capacity((cells as usize) * 20);
    for i in 0..cells {
        let dots = (filled - (i as i64) * 6).clamp(0, 6) as usize;
        let mask = PREFIX_MASKS[dots];
        if mask == 0 {
            out.push_str(&ansi_fg(muted));
        } else {
            let t = if cells > 1 { i as f64 / (cells - 1) as f64 } else { 0.0 };
            out.push_str(&ansi_fg(ramp_color(r, t)));
        }
        out.push(glyph(mask));
    }
    out.push_str("\x1b[0m");
    Ok(out)
}

/// Flattened ramp stops (at‰, r,g,b per stop) for the vitest parity test.
pub fn ramp_stops(ramp_name: &str) -> Result<Vec<i32>, String> {
    let r = find_ramp(ramp_name).ok_or_else(|| format!("unknown ramp: {ramp_name}"))?;
    let mut out = Vec::with_capacity(r.stops.len() * 4);
    for s in &r.stops {
        // at as fixed-point (x1000) to survive the i32 channel
        out.push((s.at * 1000.0).round() as i32);
        out.push(s.color.0 as i32);
        out.push(s.color.1 as i32);
        out.push(s.color.2 as i32);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_masks_fill_bottom_left_first() {
        assert_eq!(PREFIX_MASKS[0], 0x00);
        assert_eq!(PREFIX_MASKS[1], 0x04);
        assert_eq!(PREFIX_MASKS[6], 0x3f);
    }

    #[test]
    fn glyph_is_braille_base_plus_mask() {
        assert_eq!(glyph(0x3f), '\u{283f}');
        assert_eq!(glyph(0x00), '\u{2800}');
    }

    #[test]
    fn gauge_endpoints() {
        let g0 = render_gauge("activity", 0.0, 8, (0x8b, 0x94, 0x9e)).unwrap();
        // all cells empty → muted color, blank braille glyphs, single reset
        assert_eq!(g0.matches("\u{2800}").count(), 8);
        assert!(g0.ends_with("\x1b[0m"));
        let g1 = render_gauge("activity", 1.0, 8, (0, 0, 0)).unwrap();
        // all cells full → no blank glyphs
        assert_eq!(g1.matches('\u{2800}').count(), 0);
    }

    #[test]
    fn gauge_matches_ts_reference_shape() {
        // renderGauge("activity", 0.5, 4) → filled = round(0.5*4*6) = 12 →
        // cells: 6,6,0,0 → masks 3f,3f,00,00; t = 0, 1/3, 0, 0
        let g = render_gauge("activity", 0.5, 4, (1, 2, 3)).unwrap();
        let chars: Vec<char> = g.chars().collect();
        let glyphs: Vec<char> = chars.iter().copied().filter(|c| (*c as u32) >= 0x2800 && (*c as u32) <= 0x28ff).collect();
        assert_eq!(glyphs, vec!['\u{283f}', '\u{283f}', '\u{2800}', '\u{2800}']);
    }

    #[test]
    fn unknown_ramp_errors() {
        assert!(render_gauge("nope", 0.5, 4, (0, 0, 0)).is_err());
        assert!(ramp_stops("nope").is_err());
    }

    #[test]
    fn ramp_stops_capacity_ordering() {
        let stops = ramp_stops("capacity").unwrap();
        assert_eq!(stops.len(), 20);
        assert_eq!(&stops[0..4], &[0, 255, 255, 255]);
        assert_eq!(&stops[16..20], &[1000, 0x8a, 0x15, 0x15]);
    }
}
