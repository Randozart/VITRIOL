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
            "CONVERSATION",
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

    #[test]
    fn command_is_the_existing_prompt_path() {
        let mut popup = ModePopup::open(modes(), Some("grind".to_string()));
        popup.move_down(); // steer
        assert_eq!(popup.command().as_deref(), Some("/agent steer"));
    }
}
