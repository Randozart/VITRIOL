//! Tunables. Defaults live in `Config::default()`; a TOML at
//! `~/.vitriol/governor.toml` overrides any subset.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Pressure: any GPU with free VRAM below this (MiB) counts as pressured.
    pub floor_free_mib: u64,
    /// Recovery: ALL GPUs above this (MiB) free for `recover_secs` walks back.
    pub ceiling_free_mib: u64,
    /// A dip must persist this long before acting (anti-flap, fast path).
    pub confirm_secs: u64,
    /// Good conditions must persist this long before restoring.
    pub recover_secs: u64,
    /// Minimum time between transitions, both directions.
    pub min_dwell_secs: u64,
    /// Profile loaded when yielding.
    pub yield_profile: String,
    /// Profile loaded when restoring.
    pub full_profile: String,
    /// Engine HTTP port (localhost).
    pub engine_port: u16,
    /// Project root holding scripts/vitriol.
    pub project_dir: String,
    /// systemd unit name for the engine.
    pub engine_unit: String,
    /// Seconds to wait for /health after a restart.
    pub health_timeout_secs: u64,
    /// Persisted FSM state path.
    pub state_path: String,
    /// Foreign compute proc must hold at least this much VRAM (MiB) to count
    /// as a tenant; desktop compositors/browser contexts stay below it.
    pub tenant_min_mib: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            floor_free_mib: 1536,
            ceiling_free_mib: 4096,
            confirm_secs: 10,
            recover_secs: 300,
            min_dwell_secs: 600,
            yield_profile: "qwen38-buffedmod-208k".into(),
            full_profile: "qwen38-buffedmod-262k".into(),
            engine_port: 8279,
            project_dir: format!("{}/Desktop/Projects/VITRIOL", std::env::var("HOME").unwrap_or_default()),
            engine_unit: "vitriol-server".into(),
            health_timeout_secs: 120,
            state_path: "/var/lib/vitriol-governor/state.json".into(),
            tenant_min_mib: 768,
        }
    }
}

impl Config {
    pub fn load(path: &str) -> Result<Config, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("read {path}: {e}"))?;
        toml::from_str(&raw).map_err(|e| format!("parse {path}: {e}"))
    }

    /// Load `path` if it exists, else defaults.
    pub fn load_or_default(path: &str) -> Config {
        Config::load(path).unwrap_or_else(|e| {
            eprintln!("governor: config fallback to defaults ({e})");
            Config::default()
        })
    }
}
