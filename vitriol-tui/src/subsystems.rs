//! The SUBSYSTEMS diagnostic row model (Tria Prima + non-port alchemical layers).
//!
//! Rows are grouped into the three live services (whose liveness comes from the
//! latest poller snapshot) and the four logical layers that have no port of their
//! own (Spagyric, Chimera, Ascensus, Copula). Each row carries an alchemical
//! glyph, a human name, a status, a short live value, and the config keys that
//! affect it (read from the active `~/.vitriol/config` INI and the environment).
//! Everything here is read-only: the SUBSYSTEMS tab is diagnostic, never spawns.

use std::collections::HashMap;

use crate::config::Config;
use crate::model::Snapshot;
use crate::theme;

/// Liveness colour for a subsystem row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Service reachable / feature active.
    Up,
    /// Service down / feature disabled.
    Down,
    /// Cannot tell from the given inputs.
    Unknown,
}

/// One SUBSYSTEMS tab row.
#[derive(Debug, Clone)]
pub struct Row {
    /// Alchemical glyph (Tria Prima / layer emblem).
    pub glyph: &'static str,
    /// Display name, e.g. "HERMETIS".
    pub name: &'static str,
    /// One-line current live value, e.g. "7980 · 12 episodes".
    pub value: String,
    /// Liveness.
    pub status: Status,
    /// Config keys that drive this layer (`section.key`).
    pub config: Vec<&'static str>,
    /// Section ordering group: 0 = port trio, 1 = logical layers.
    pub group: u8,
}

/// Group ordinal for the port-trio rows.
pub const GROUP_SERVICES: u8 = 0;
/// Group ordinal for the non-port logical-layer rows.
pub const GROUP_LAYERS: u8 = 1;

/// Build the full row set from the live snapshot and config.
pub fn rows(cfg: &Config, snap: &Snapshot) -> Vec<Row> {
    let services = service_rows(cfg, snap);
    let layers = layer_rows(cfg);
    let mut out = services;
    out.extend(layers);
    out
}

/// The three port-bearing services (gen / hermetis / embed), liveness from the
/// latest poll.
fn service_rows(cfg: &Config, snap: &Snapshot) -> Vec<Row> {
    let gen = if snap.gen.up {
        (
            Status::Up,
            format!("{} · {} t/s", cfg.gen_port, snap.gen.decode_t_s),
        )
    } else {
        (Status::Down, cfg.gen_port.to_string())
    };
    let herm_episodes = snap.hermetis.episodes;
    let herm = if snap.hermetis.up {
        let ep = herm_episodes.map_or_else(|| "?".to_string(), |n| n.to_string());
        (Status::Up, format!("{} · {ep} episodes", cfg.hermetis_port))
    } else {
        (Status::Down, cfg.hermetis_port.to_string())
    };
    let embed = if snap.embed.up {
        (Status::Up, cfg.embed_port.to_string())
    } else {
        (Status::Down, cfg.embed_port.to_string())
    };
    vec![
        Row {
            glyph: theme::GLYPH_GEN,
            name: "GEN",
            value: gen.1,
            status: gen.0,
            config: vec!["server.port", "vitriol.mode"],
            group: GROUP_SERVICES,
        },
        Row {
            glyph: theme::GLYPH_HERM,
            name: "HERMETIS",
            value: herm.1,
            status: herm.0,
            config: vec!["memory.mode", "memory.semantic_mode"],
            group: GROUP_SERVICES,
        },
        Row {
            glyph: theme::GLYPH_EMBED,
            name: "EMBED",
            value: embed.1,
            status: embed.0,
            config: vec!["memory.semantic_mode"],
            group: GROUP_SERVICES,
        },
    ]
}

/// The non-port alchemical layers, config and env-driven.
fn layer_rows(cfg: &Config) -> Vec<Row> {
    let kv = load_config(cfg);
    let ascensus_configured = !std::env::var("GEMINI_API_KEY")
        .map(|k| k.trim().is_empty())
        .unwrap_or(true);
    vec![
        Row {
            glyph: "♄",
            name: "SPAGYRIC",
            value: "autotuner·sweep".into(),
            status: status_of(&kv, &["vitriol.mode"], Status::Unknown),
            config: vec!["[spagyric] profile", "vitriol.mode"],
            group: GROUP_LAYERS,
        },
        Row {
            glyph: "☉",
            name: "CHIMERA",
            value: "dual-backend routing".into(),
            status: status_of(&kv, &["engine"], Status::Unknown),
            config: vec!["engine", "model.expert_count"],
            group: GROUP_LAYERS,
        },
        Row {
            glyph: "♀",
            name: "ASCENSUS",
            value: if ascensus_configured {
                "cloud escalation · key set".into()
            } else {
                "cloud escalation · no key".into()
            },
            status: if ascensus_configured {
                Status::Up
            } else {
                Status::Down
            },
            config: vec!["GEMINI_API_KEY"],
            group: GROUP_LAYERS,
        },
        Row {
            glyph: "⚳",
            name: "COPULA",
            value: copula_value(cfg),
            status: copula_status(cfg),
            config: vec!["COPULA_ENABLED"],
            group: GROUP_LAYERS,
        },
    ]
}

/// Liveness from a config key: present+enabled => Up, present+disabled => Down.
fn status_of(kv: &HashMap<String, String>, keys: &[&str], fallback: Status) -> Status {
    for key in keys {
        if let Some(val) = kv.get(*key) {
            return match val.to_ascii_lowercase().as_str() {
                "on" | "yes" | "true" | "1" | "auto" => Status::Up,
                "off" | "no" | "false" | "0" => Status::Down,
                _ => Status::Unknown,
            };
        }
    }
    fallback
}

/// Copula enabled flag and its render value.
fn copula_value(cfg: &Config) -> String {
    let on = std::env::var("COPULA_ENABLED")
        .map(|v| v != "0")
        .unwrap_or(true);
    let url = cfg.hermetis_base();
    if on {
        format!("bond active · {url}")
    } else {
        format!("disabled · {url}")
    }
}

fn copula_status(cfg: &Config) -> Status {
    let _ = cfg;
    let on = std::env::var("COPULA_ENABLED")
        .map(|v| v != "0")
        .unwrap_or(true);
    if on {
        Status::Up
    } else {
        Status::Down
    }
}

/// Read the active `~/.vitriol/config` INI into a `section.key -> value` map.
///
/// Reuses the same flattening rules as `profile::parse_ini`; a missing config
/// file yields an empty map (every layer shows Unknown, never panics).
fn load_config(cfg: &Config) -> HashMap<String, String> {
    let path = cfg.home_dir.join(".vitriol").join("config");
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    parse_flattened(&text)
}

/// Flatten an INI into `section.key -> value`.
fn parse_flattened(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut section = "";
    for line in text.lines() {
        let line = line.trim();
        if skip_line(line) {
            continue;
        }
        if let Some(close) = section_of(line) {
            section = close;
            continue;
        }
        insert_key(&mut out, line, section);
    }
    out
}

/// True for blank lines and comments (begins with `#`).
fn skip_line(line: &str) -> bool {
    line.is_empty() || line.starts_with('#')
}

/// The section name when `line` is a `[section]` header; else `None`.
fn section_of(line: &str) -> Option<&str> {
    let stripped = line.strip_prefix('[')?;
    let close = stripped.strip_suffix(']')?;
    let name = close.trim();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Insert one `k=v` line into the map under the active section.
fn insert_key(out: &mut HashMap<String, String>, line: &str, section: &str) {
    let Some((raw_k, v)) = line.split_once('=') else {
        return;
    };
    let k = raw_k.trim();
    if k.is_empty() {
        return;
    }
    let key = if section.is_empty() {
        k.to_string()
    } else {
        format!("{section}.{}", k)
    };
    out.insert(key, v.trim().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Snapshot;

    #[test]
    fn service_rows_reflect_port_trio() {
        let mut cfg = Config::from_env();
        cfg.home_dir = "/nonexistent".into();
        let rows = rows(&cfg, &Snapshot::default());
        assert_eq!(rows.len(), 7);
        let gen = &rows[0];
        assert_eq!(gen.name, "GEN");
        assert_eq!(gen.status, Status::Down);
        // Comfortable value encoding; assert it carries the port.
        assert!(gen.value.contains(&cfg.gen_port.to_string()));
    }

    #[test]
    fn parse_flattened_handles_sections_and_comments() {
        let text = "# c\n[model]\npath = /x\nngl = 24\n\n[server]\nparallel=4\n";
        let kv = parse_flattened(text);
        assert_eq!(kv.get("model.path").map(String::as_str), Some("/x"));
        assert_eq!(kv.get("server.parallel").map(String::as_str), Some("4"));
        assert_eq!(kv.len(), 3);
    }

    #[test]
    fn missing_config_yields_unknown_layers_not_crash() {
        let mut cfg = Config::from_env();
        cfg.home_dir = "/nonexistent".into();
        let kv = load_config(&cfg);
        assert!(kv.is_empty());
    }
}
