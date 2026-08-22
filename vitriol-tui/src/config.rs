//! Endpoints, log locations, and identity for VITRIOL services.
//!
//! Defaults mirror `scripts/launch_vitriol_full.sh` + `scripts/vitriol-ports.sh`
//! (Tria Prima): gen 8279, Hermetis 7980, embed 4779, logs under `$COPULA_LOG_DIR`
//! (default `/tmp/opencode`). Every value is overridable via the same environment
//! variables the launch script honours, so the TUI always talks to the stack the
//! script owns.

use std::env;
use std::path::PathBuf;

/// VITRIOL service endpoints and log paths.
#[derive(Clone)]
pub struct Config {
    /// Gen (llama-server) HTTP port. Env `VITRIOL_GEN_PORT`.
    pub gen_port: u16,
    /// Hermetis memory server port. Env `VITRIOL_HERM_PORT`.
    pub hermetis_port: u16,
    /// Embed (bge) server port. Env `VITRIOL_EMBED_PORT`.
    pub embed_port: u16,
    /// Luna head port (Mellum2). Env `VITRIOL_LUNA_PORT`.
    pub luna_port: u16,
    /// Mercury gateway port (REBIS). Env `VITRIOL_GATEWAY_PORT`.
    pub gateway_port: u16,
    /// Distillation store directory. Env `VITRIOL_DISTILL_DIR`.
    pub distill_dir: PathBuf,
    /// Directory holding the service log files. Env `COPULA_LOG_DIR`.
    pub log_dir: PathBuf,
    /// Hermetis project id for `/hermetis/stats`. Env `VITRIOL_PROJECT_ID`,
    /// defaulting to the basename of the working directory.
    pub project_id: String,
    /// VITRIOL repository root (where `scripts/` and `profiles/` live).
    /// Env `VITRIOL_REPO`, else the working directory when it contains the
    /// launch script, else the working directory.
    pub repo_root: PathBuf,
    /// User home directory (for `~/.vitriol`). Env `HOME`.
    pub home_dir: PathBuf,
}

impl Config {
    /// Build config from the process environment with launch-script defaults.
    pub fn from_env() -> Self {
        let gen_port = env::var("VITRIOL_GEN_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8279);
        let hermetis_port = env::var("VITRIOL_HERM_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(7980);
        let embed_port = env::var("VITRIOL_EMBED_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(4779);
        let luna_port = env::var("VITRIOL_LUNA_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8247);
        let gateway_port = env::var("VITRIOL_GATEWAY_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8280);
        let distill_dir = env::var_os("VITRIOL_DISTILL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env::var("HOME").unwrap_or_default())
                    .join(".vitriol/distill")
            });
        let log_dir = env::var_os("COPULA_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp/opencode"));
        let project_id = env::var("VITRIOL_PROJECT_ID")
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(default_project_id);
        let repo_root = env::var_os("VITRIOL_REPO")
            .map(PathBuf::from)
            .or_else(detect_repo_root)
            .unwrap_or_else(current_dir);
        let home_dir = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(current_dir);
        Self {
            gen_port,
            hermetis_port,
            embed_port,
            luna_port,
            gateway_port,
            distill_dir,
            log_dir,
            project_id,
            repo_root,
            home_dir,
        }
    }

    /// Absolute path of the gen server log.
    pub fn gen_log(&self) -> PathBuf {
        self.log_dir.join("vitriol_gen.log")
    }

    /// Absolute path of the Hermetis log.
    pub fn hermetis_log(&self) -> PathBuf {
        self.log_dir.join("copula_hermetis.log")
    }

    /// Absolute path of the embed server log.
    pub fn embed_log(&self) -> PathBuf {
        self.log_dir.join("copula_embed.log")
    }

    /// Absolute path of the full-stack launch script.
    pub fn launch_script(&self) -> PathBuf {
        self.repo_root.join("scripts/launch_vitriol_full.sh")
    }

    /// Absolute path of the `vitriol` config CLI.
    pub fn vitriol_cli(&self) -> PathBuf {
        self.repo_root.join("scripts/vitriol")
    }

    /// Directory of bundled (repo) profiles.
    pub fn bundled_profiles_dir(&self) -> PathBuf {
        self.repo_root.join("profiles")
    }

    /// Directory of installed (user) profiles under `~/.vitriol`.
    pub fn installed_profiles_dir(&self) -> PathBuf {
        self.home_dir.join(".vitriol/profiles")
    }

    /// Path of the Ascensus secrets file (`~/.vitriol/secrets`). This file
    /// holds the Gemini API key + model and is NEVER committed to the repo.
    pub fn secrets_path(&self) -> PathBuf {
        self.home_dir.join(".vitriol/secrets")
    }

    /// Base URL of the gen server.
    pub fn gen_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.gen_port)
    }

    /// Base URL of the Hermetis server.
    pub fn hermetis_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.hermetis_port)
    }

    /// Base URL of the embed server.
    pub fn embed_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.embed_port)
    }
}

/// Detect the repo root as the current directory when it contains the launch
/// script, walking up at most a couple of levels for nested invocations.
fn detect_repo_root() -> Option<PathBuf> {
    let mut dir = current_dir();
    for _ in 0..3 {
        if dir.join("scripts/launch_vitriol_full.sh").is_file() {
            return Some(dir);
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            break;
        }
    }
    None
}

/// Current directory as a fallback path source.
fn current_dir() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Fallback Hermetis project id: the full working-directory path sanitized the
/// same way `hermetis_server._project_id` sanitizes (`/` -> `_`, 120-char
/// cap), so the TUI reads the same project the Copula plugin writes.
fn default_project_id() -> String {
    let raw = env::current_dir()
        .ok()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("default"));
    let sanitized = raw.replace(['/', '\\', ':'], "_");
    sanitized.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// Env vars are process-global, so the two env tests must not run in
    /// parallel or they clobber each other's assignments.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn env_overrides_all_fields() {
        let _guard = env_lock().lock().unwrap();
        env::set_var("VITRIOL_GEN_PORT", "9000");
        env::set_var("VITRIOL_HERM_PORT", "9100");
        env::set_var("VITRIOL_EMBED_PORT", "9200");
        env::set_var("COPULA_LOG_DIR", "/tmp/env-test-logs");
        env::set_var("VITRIOL_PROJECT_ID", "proj-x");
        let cfg = Config::from_env();
        assert_eq!(cfg.gen_port, 9000);
        assert_eq!(cfg.hermetis_port, 9100);
        assert_eq!(cfg.embed_port, 9200);
        assert_eq!(cfg.log_dir, PathBuf::from("/tmp/env-test-logs"));
        assert_eq!(cfg.project_id, "proj-x");
        assert_eq!(
            cfg.gen_log(),
            PathBuf::from("/tmp/env-test-logs/vitriol_gen.log")
        );
        env::remove_var("VITRIOL_GEN_PORT");
        env::remove_var("VITRIOL_HERM_PORT");
        env::remove_var("VITRIOL_EMBED_PORT");
        env::remove_var("COPULA_LOG_DIR");
        env::remove_var("VITRIOL_PROJECT_ID");
    }

    #[test]
    fn defaults_when_env_unset() {
        let _guard = env_lock().lock().unwrap();
        env::remove_var("VITRIOL_GEN_PORT");
        env::remove_var("VITRIOL_HERM_PORT");
        env::remove_var("VITRIOL_EMBED_PORT");
        env::remove_var("COPULA_LOG_DIR");
        env::remove_var("VITRIOL_PROJECT_ID");
        let cfg = Config::from_env();
        assert_eq!(cfg.gen_port, 8279);
        assert_eq!(cfg.hermetis_port, 7980);
        assert_eq!(cfg.embed_port, 4779);
        assert_eq!(cfg.log_dir, PathBuf::from("/tmp/opencode"));
    }

    #[test]
    fn default_project_id_is_sanitized_full_path() {
        // Must be deterministic regardless of the actual cwd.
        let cwd = env::current_dir().unwrap();
        let expected = cwd
            .to_string_lossy()
            .replace(['/', '\\', ':'], "_")
            .chars()
            .take(120)
            .collect::<String>();
        let _guard = env_lock().lock().unwrap();
        env::remove_var("VITRIOL_PROJECT_ID");
        assert_eq!(default_project_id(), expected);
        assert!(!default_project_id().contains('/'));
    }
}
