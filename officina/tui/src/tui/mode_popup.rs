//! TAB mode popup (2026-11-19, owner request): with 8 modes, blind TAB
//! cycling is no longer navigation — it is roulette. TAB on an EMPTY
//! composer opens this popup; arrows + Enter pick a mode, TAB/TAB or Esc
//! dismisses it. TAB with text in the composer keeps the historical cycle
//! (invariant I10 — muscle memory preserved, pinned by tests).
//!
//! The mode list comes from `~/.vitriol/officina/agent-mode.json` — the facts
//! file the agent-mode extension already publishes (since 2026-11-19 it
//! carries the full ordered mode list, not just the current mode). Missing
//! or stale file → a built-in fallback list so the popup never goes dark
//! (invariant I11). Enter sends `/agent <name>` via the existing prompt path
//! — no protocol change.

use serde::Deserialize;

/// One mode as listed in the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeEntry {
    pub name: String,
    pub label: String,
    pub glyph: String,
    pub color: String,
    pub hint: String,
}

/// The facts file the agent-mode extension publishes (single source for both
/// the popup list and the chip colours).
pub fn facts_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".vitriol")
        .join("officina")
        .join("agent-mode.json")
}

/// Built-in fallback when agent-mode.json is missing/stale. Kept in lockstep
/// with `_shared/agent-mode.ts` DEFAULT_MODES (I11).
pub fn fallback_modes() -> Vec<ModeEntry> {
    [
        (
            "build",
            "BUILD",
            "▪",
            "#2e5fa3",
            "writes unlocked · TAB / /mode plan for research",
        ),
        (
            "grind",
            "GRIND",
            "∞",
            "#b294bb",
            "persistent build · runs until GRIND: DONE · TAB / /agent build",
        ),
        (
            "steer",
            "STEER",
            "↝",
            "#ffa000",
            "redirect the work in flight · keep going after the correction · TAB / /agent build",
        ),
        (
            "inquire",
            "INQUIRE",
            "?",
            "#3fb950",
            "answers & steering only · work keeps running · TAB / /agent build",
        ),
        ("plan", "PLAN", "►", "#ffd700", "research only, *.md writes"),
        (
            "conversation",
            "DISCUSS",
            "◇",
            "#2fa08a",
            "repo chat & light research · no state changes · TAB / /agent build",
        ),
        (
            "research",
            "RESEARCH",
            "∿",
            "#3a74d4",
            "deep online research · *.md writes only · TAB / /agent plan",
        ),
        (
            "pipeline",
            "PIPELINE",
            "⟲",
            "#e05adf",
            "spec-driven: research → confirm → plan → grind · compacts after",
        ),
    ]
    .into_iter()
    .map(|(name, label, glyph, color, hint)| ModeEntry {
        name: name.to_string(),
        label: label.to_string(),
        glyph: glyph.to_string(),
        color: color.to_string(),
        hint: hint.to_string(),
    })
    .collect()
}

#[derive(Debug, Deserialize)]
struct FactsFile {
    mode: Option<String>,
    #[serde(default)]
    modes: Vec<FactsMode>,
}

#[derive(Debug, Deserialize)]
struct FactsMode {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    glyph: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    hint: Option<String>,
}

/// Load the mode list + current mode from the facts file. Falls back to
/// `fallback_modes()` when the file is missing, unparsable, or lists fewer
/// than two modes (a list without a choice is not a menu).
pub fn load_modes(path: &std::path::Path) -> (Vec<ModeEntry>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (fallback_modes(), None);
    };
    let Ok(facts) = serde_json::from_str::<FactsFile>(&text) else {
        return (fallback_modes(), None);
    };
    let modes: Vec<ModeEntry> = facts
        .modes
        .into_iter()
        .map(|mut m| ModeEntry {
            label: m.label.take().unwrap_or_else(|| m.name.clone()),
            glyph: m.glyph.take().unwrap_or_else(|| "▪".to_string()),
            color: m.color.take().unwrap_or_default(),
            hint: m.hint.take().unwrap_or_default(),
            name: m.name,
        })
        .collect();
    if modes.len() < 2 {
        return (fallback_modes(), facts.mode);
    }
    (modes, facts.mode)
}

/// Popup selection state. `selected` indexes into the mode list; arrows wrap.
#[derive(Debug, Clone)]
pub struct ModePopup {
    pub modes: Vec<ModeEntry>,
    pub current: Option<String>,
    pub selected: usize,
}

impl ModePopup {
    /// Open with the given list, selecting the current mode (or 0).
    pub fn open(modes: Vec<ModeEntry>, current: Option<String>) -> Self {
        let selected = modes
            .iter()
            .position(|m| Some(&m.name) == current.as_ref())
            .unwrap_or(0);
        Self {
            modes,
            current,
            selected,
        }
    }

    pub fn move_up(&mut self) {
        if self.modes.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.modes.len() - 1
        } else {
            self.selected - 1
        };
    }

    pub fn move_down(&mut self) {
        if self.modes.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.modes.len();
    }

    /// The mode the operator picked, if any.
    pub fn picked(&self) -> Option<&ModeEntry> {
        self.modes.get(self.selected)
    }

    /// The command line Enter should submit for this mode.
    pub fn command(&self) -> Option<String> {
        self.picked().map(|m| format!("/agent {}", m.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes() -> Vec<ModeEntry> {
        fallback_modes()
    }

    /// I11: missing/stale facts file degrades to the built-in list — the
    /// popup never goes dark.
    #[test]
    fn missing_facts_file_falls_back() {
        let (modes, current) = load_modes(std::path::Path::new("/nonexistent/agent-mode.json"));
        assert_eq!(modes.len(), 8);
        assert_eq!(modes[0].name, "build");
        assert_eq!(current, None);
    }

    #[test]
    fn stale_facts_file_falls_back() {
        let dir = std::env::temp_dir().join("officina-mode-popup-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.json");
        std::fs::write(&path, "{ not json").unwrap();
        let (modes, _) = load_modes(&path);
        assert_eq!(modes.len(), 8);
    }

    #[test]
    fn facts_file_with_full_list_is_used() {
        let dir = std::env::temp_dir().join("officina-mode-popup-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("full.json");
        std::fs::write(
            &path,
            r##"{"version":1,"ts":1,"mode":"plan","modes":[
              {"name":"build","label":"BUILD","glyph":"▪","color":"#2e5fa3","hint":"h1"},
              {"name":"plan","label":"PLAN","glyph":"►","color":"#ffd700","hint":"h2"}]}"##,
        )
        .unwrap();
        let (modes, current) = load_modes(&path);
        assert_eq!(modes.len(), 2);
        assert_eq!(current.as_deref(), Some("plan"));
    }

    /// I10 companion: opening selects the CURRENT mode, so Enter on an
    /// untouched popup is a no-op switch (never a surprise mode change).
    #[test]
    fn open_selects_the_current_mode() {
        let mut popup = ModePopup::open(modes(), Some("plan".to_string()));
        assert_eq!(popup.selected, 4); // plan is 5th in the fallback list
        assert_eq!(popup.command().as_deref(), Some("/agent plan"));
    }

    #[test]
    fn arrows_wrap_both_ways() {
        let mut popup = ModePopup::open(modes(), None); // build, index 0
        popup.move_up();
        assert_eq!(popup.picked().unwrap().name, "pipeline"); // wrapped to last
        popup.move_down();
        assert_eq!(popup.picked().unwrap().name, "build"); // wrapped to first
        popup.move_down();
        assert_eq!(popup.picked().unwrap().name, "grind");
    }

    /// 2026-11-21: the chip must read a mode's DECLARED colour, not fall back
    /// to BUILD gold for every mode added after the original five.
    #[test]
    fn declared_colours_are_parsed_from_the_facts_file() {
        let dir = std::env::temp_dir().join("officina-mode-popup-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("colours.json");
        std::fs::write(
            &path,
            r##"{"version":1,"ts":1,"mode":"pipeline","modes":[
              {"name":"pipeline","label":"PIPELINE","color":"#e05adf"},
              {"name":"research","label":"RESEARCH","color":"#3a74d4"},
              {"name":"build","label":"BUILD","color":"coldBlue"}]}"##,
        )
        .unwrap();
        assert_eq!(hex_for_label("PIPELINE", &path), Some((0xe0, 0x5a, 0xdf)));
        assert_eq!(hex_for_label("research", &path), Some((0x3a, 0x74, 0xd4)));
        // A named Vitriolum colour has no hex — the caller uses theme roles.
        assert_eq!(hex_for_label("BUILD", &path), None);
        assert_eq!(parse_hex_colour("#zzzzzz"), None);
        assert_eq!(parse_hex_colour("e05adf"), None);
    }

    #[test]
    fn command_is_the_existing_prompt_path() {
        let mut popup = ModePopup::open(modes(), Some("grind".to_string()));
        popup.move_down(); // steer
        assert_eq!(popup.command().as_deref(), Some("/agent steer"));
    }
}

// ── Per-mode chip colour (2026-11-21) ─────────────────────────────────────
// The composer chip used to know only the five original modes; the three
// newer ones fell through to the BUILD gold. The facts file already carries
// each mode's declared colour (the extension writes it), so the chip reads
// THAT instead of duplicating the table in Rust. Named Vitriolum colours
// (coldBlue, violet, …) still resolve through the theme table in layout.rs —
// only hex values are handled here.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static MODE_COLOURS: OnceLock<Mutex<ModeColourCache>> = OnceLock::new();

#[derive(Default)]
struct ModeColourCache {
    /// File mtime the cache was built from.
    mtime: Option<std::time::SystemTime>,
    /// Checked-at instant: the file is stat-ed at most this often, never per
    /// frame (the chip renders every frame).
    checked_at: Option<Instant>,
    /// label (uppercased) → rgb
    colours: std::collections::HashMap<String, (u8, u8, u8)>,
}

/// Re-stat cadence for the facts file. A mode switch happens on a human
/// timescale; 1 s is far below perception and far above frame rate.
const MODE_COLOUR_RECHECK: Duration = Duration::from_secs(1);

fn parse_hex_colour(s: &str) -> Option<(u8, u8, u8)> {
    let t = s.trim();
    let hex = t.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some((
        ((n >> 16) & 0xff) as u8,
        ((n >> 8) & 0xff) as u8,
        (n & 0xff) as u8,
    ))
}

/// Declared colour for a mode LABEL ("DISCUSS", "PIPELINE", …), from the
/// agent-mode facts file. `None` when the file is missing, stale-ish, or the
/// mode declares a named colour (the caller falls back to the theme table).
pub fn hex_for_label(label: &str, path: &std::path::Path) -> Option<(u8, u8, u8)> {
    let cache = MODE_COLOURS.get_or_init(|| Mutex::new(ModeColourCache::default()));
    let Ok(mut guard) = cache.lock() else {
        return None;
    };
    let now = Instant::now();
    let due = guard
        .checked_at
        .is_none_or(|at| now.duration_since(at) >= MODE_COLOUR_RECHECK);
    if due {
        guard.checked_at = Some(now);
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime != guard.mtime {
            guard.mtime = mtime;
            guard.colours = read_mode_colours(path);
        }
    }
    guard.colours.get(&label.to_ascii_uppercase()).copied()
}

/// Parse the facts file into label → hex. Tolerant by contract: a broken file
/// yields an empty map (the chip falls back to the theme table), never an
/// error the renderer would have to handle.
fn read_mode_colours(path: &std::path::Path) -> std::collections::HashMap<String, (u8, u8, u8)> {
    let mut out = std::collections::HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    let Ok(facts) = serde_json::from_str::<FactsFile>(&text) else {
        return out;
    };
    for m in facts.modes {
        let label = m.label.clone().unwrap_or_else(|| m.name.clone());
        if let Some(hex) = m.color.as_deref().and_then(parse_hex_colour) {
            out.insert(label.to_ascii_uppercase(), hex);
        }
    }
    out
}
