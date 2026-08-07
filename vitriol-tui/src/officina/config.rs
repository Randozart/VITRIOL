//! Officina REPL configuration (`~/.vitriol/officina.toml`).
//!
//! Controls which telemetry blocks the ALKA-☿ prompt renders and the journal
//! sidebar width. Parsed as a small TOML subset (sections + `key = value` with
//! bool/int/string values); missing keys fall back to defaults, a missing file
//! yields the default configuration (never errors).

use std::path::Path;

/// Officina REPL configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficinaConfig {
    /// Show the active model + dirty state block.
    pub show_model: bool,
    /// Show the context block.
    pub show_context: bool,
    /// Show the cumulative estimated drift block.
    pub show_drift: bool,
    /// Show the VRAM residency block.
    pub show_vram: bool,
    /// Show the MoE active-expert block.
    pub show_experts: bool,
    /// Prompt line-art theme (only `kali-teal` ships).
    pub theme: String,
    /// Render ALKA in bold.
    pub bold_logo: bool,
    /// Right-hand journal sidebar width in columns.
    pub sidebar_width: usize,
}

impl Default for OfficinaConfig {
    fn default() -> Self {
        Self {
            show_model: true,
            show_context: true,
            show_drift: true,
            show_vram: false,
            show_experts: false,
            theme: "kali-teal".into(),
            bold_logo: true,
            sidebar_width: 35,
        }
    }
}

impl OfficinaConfig {
    /// Load from `path`, overlaying defaults with any present keys.
    pub fn load(path: &Path) -> Self {
        let mut cfg = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return cfg;
        };
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(s) = section_of(line) {
                section = s;
                continue;
            }
            cfg.apply_key(&section, line);
        }
        cfg
    }

    /// Apply one `key = value` line from the named section.
    fn apply_key(&mut self, section: &str, line: &str) {
        let Some((k, v)) = line.split_once('=') else {
            return;
        };
        let k = k.trim();
        let v = v.trim();
        match section {
            "repl.telemetry" => match k {
                "show_model" => self.show_model = parse_bool(v, self.show_model),
                "show_context" => self.show_context = parse_bool(v, self.show_context),
                "show_drift" => self.show_drift = parse_bool(v, self.show_drift),
                "show_vram" => self.show_vram = parse_bool(v, self.show_vram),
                "show_experts" => self.show_experts = parse_bool(v, self.show_experts),
                _ => {}
            },
            "repl.style" => match k {
                "theme" => {
                    let v = v.trim().trim_matches('"').trim();
                    if !v.is_empty() {
                        self.theme = v.to_string();
                    }
                }
                "bold_logo" => self.bold_logo = parse_bool(v, self.bold_logo),
                "sidebar_width" => {
                    if let Ok(w) = v.parse::<usize>() {
                        self.sidebar_width = w.max(20);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// The trimmed section name when `line` is a `[section]` header, else empty.
fn section_of(line: &str) -> Option<String> {
    let stripped = line.strip_prefix('[')?;
    let close = stripped.strip_suffix(']')?;
    let name = close.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Parse a boolean-ish value, falling back to `default` on garbage.
fn parse_bool(v: &str, default: bool) -> bool {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => true,
        "false" | "no" | "0" | "off" => false,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let cfg = OfficinaConfig::load(Path::new("/nonexistent/officina.toml"));
        assert!(cfg.show_model);
        assert!(cfg.bold_logo);
        assert_eq!(cfg.sidebar_width, 35);
    }

    #[test]
    fn parses_sections_and_overrides() {
        let p = std::env::temp_dir().join("officina_test.toml");
        std::fs::write(
            &p,
            "[repl.telemetry]\nshow_model = false\nshow_vram = true\n\n[repl.style]\ntheme = \"neon\"\nsidebar_width = 45\nbold_logo = false\n",
        )
        .unwrap();
        let cfg = OfficinaConfig::load(&p);
        assert!(!cfg.show_model);
        assert!(cfg.show_vram);
        assert_eq!(cfg.theme, "neon");
        assert_eq!(cfg.sidebar_width, 45);
        assert!(!cfg.bold_logo);
        assert!(cfg.show_context);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn garbage_values_fall_back() {
        let p = std::env::temp_dir().join("officina_bad.toml");
        std::fs::write(&p, "[repl.telemetry]\nshow_drift = banana\n").unwrap();
        let cfg = OfficinaConfig::load(&p);
        assert!(cfg.show_drift);
        let _ = std::fs::remove_file(&p);
    }
}
