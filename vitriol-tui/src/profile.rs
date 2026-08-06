//! Spagyric profile discovery and INI parsing.
//!
//! Profiles describe model launch knobs. Two locations are consulted: bundled
//! profiles under the repo `profiles/` and installed profiles under
//! `~/.vitriol/profiles`. An installed profile with the same name shadows the
//! bundled one. Each profile has a `config` INI (`[model]`, `[server]`) and a
//! `meta` file (`name=`, `description=`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// Where a profile was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSource {
    /// Repo-bundled under `profiles/<name>`.
    Bundled,
    /// User-installed under `~/.vitriol/profiles/<name>`.
    Installed,
}

/// A launch profile: model knobs extracted from its config INI.
#[derive(Debug, Clone)]
pub struct Profile {
    /// Profile directory name (also the `vitriol config load` name).
    pub name: String,
    /// Human description from `meta`.
    pub description: String,
    /// Where the profile was found.
    pub source: ProfileSource,
    /// `model.path`.
    pub model: Option<String>,
    /// `model.ngl`.
    pub ngl: Option<u32>,
    /// `model.context`.
    pub ctx: Option<u32>,
    /// `model.threads`.
    pub threads: Option<u32>,
    /// `server.parallel`.
    pub parallel: Option<u32>,
}

/// Discover all profiles (bundled + installed, installed shadowing bundled).
pub fn discover(cfg: &Config) -> Vec<Profile> {
    let mut bundled = read_dir_profiles(&cfg.bundled_profiles_dir(), ProfileSource::Bundled);
    let installed = read_dir_profiles(&cfg.installed_profiles_dir(), ProfileSource::Installed);
    for inst in installed {
        if let Some(existing) = bundled.iter_mut().find(|p| p.name == inst.name) {
            *existing = inst;
        } else {
            bundled.push(inst);
        }
    }
    bundled.sort_by(|a, b| a.name.cmp(&b.name));
    bundled
}

/// Load profiles from every subdirectory of `dir` that has a `config` file.
fn read_dir_profiles(dir: &Path, source: ProfileSource) -> Vec<Profile> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().join("config").is_file())
        .filter_map(|e| Profile::load(source, e.path()))
        .collect()
}

impl Profile {
    /// Load a profile from its directory (which must contain `config`).
    fn load(source: ProfileSource, dir: PathBuf) -> Option<Profile> {
        let name = dir.file_name()?.to_string_lossy().into_owned();
        let config_text = fs::read_to_string(dir.join("config")).ok()?;
        let description = fs::read_to_string(dir.join("meta"))
            .ok()
            .and_then(|m| meta_value(&m, "description"))
            .unwrap_or_default();
        let kv = parse_ini(&config_text);
        Some(Profile {
            name,
            description,
            source,
            model: kv.get("model.path").cloned(),
            ngl: kv.get("model.ngl").and_then(|v| v.parse().ok()),
            ctx: kv.get("model.context").and_then(|v| v.parse().ok()),
            threads: kv.get("model.threads").and_then(|v| v.parse().ok()),
            parallel: kv.get("server.parallel").and_then(|v| v.parse().ok()),
        })
    }
}

/// Parse an INI-ish config into a `section.key -> value` map. Comments and
/// blank lines are ignored; a `[section]` header prefixes subsequent keys.
fn parse_ini(text: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let mut section = "";
    for line in text.lines() {
        parse_ini_line(line, &mut section, &mut out);
    }
    out
}

/// Fold one config line into the key map, tracking the active section.
fn parse_ini_line<'a>(
    line: &'a str,
    section: &mut &'a str,
    out: &mut std::collections::HashMap<String, String>,
) {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return;
    }
    if let Some(s) = line.strip_prefix('[') {
        if let Some(s) = s.strip_suffix(']') {
            *section = s.trim();
        }
        return;
    }
    let Some((k, v)) = line.split_once('=') else {
        return;
    };
    let key = if section.is_empty() {
        k.trim().to_string()
    } else {
        format!("{section}.{}", k.trim())
    };
    out.insert(key, v.trim().to_string());
}

/// Read a `key=value` from a meta file.
fn meta_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .map(|v| v.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_parse_flattens_sections() {
        let text = "# comment\n[model]\npath = /x/model.gguf\nngl = 24\n\n[server]\nparallel = 4\n";
        let kv = parse_ini(text);
        assert_eq!(
            kv.get("model.path").map(String::as_str),
            Some("/x/model.gguf")
        );
        assert_eq!(kv.get("model.ngl").map(String::as_str), Some("24"));
        assert_eq!(kv.get("server.parallel").map(String::as_str), Some("4"));
        assert_eq!(kv.len(), 3);
    }

    #[test]
    fn meta_description_parse() {
        let m = "name=mellum2\ndescription=The mellum2 profile\ncreated=1\n";
        assert_eq!(
            meta_value(m, "description").as_deref(),
            Some("The mellum2 profile")
        );
    }
}
