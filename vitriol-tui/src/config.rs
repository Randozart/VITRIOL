//! Endpoints, log locations, and identity for VITRIOL services.
//!
//! Defaults mirror `scripts/launch_vitriol_full.sh`: gen on 8279, Hermetis on
//! 8090, embed on 8081, logs under `$COPULA_LOG_DIR` (default `/tmp/opencode`).
//! Every value is overridable via the same environment variables the launch
//! script honours, so the TUI always talks to the stack the script owns.

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
    /// Directory holding the service log files. Env `COPULA_LOG_DIR`.
    pub log_dir: PathBuf,
    /// Hermetis project id for `/hermetis/stats`. Env `VITRIOL_PROJECT_ID`,
    /// defaulting to the basename of the working directory.
    pub project_id: String,
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
            .unwrap_or(8090);
        let embed_port = env::var("VITRIOL_EMBED_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8081);
        let log_dir = env::var_os("COPULA_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp/opencode"));
        let project_id = env::var("VITRIOL_PROJECT_ID")
            .ok()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(default_project_id);
        Self {
            gen_port,
            hermetis_port,
            embed_port,
            log_dir,
            project_id,
        }
    }

    /// Absolute path of the gen server log.
    pub fn gen_log(&self) -> PathBuf {
        self.log_dir.join("vitriol_gen.log")
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

/// Fallback Hermetis project id: the basename of the current directory, which
/// matches how the Copula plugin derives a project id from the workspace.
fn default_project_id() -> String {
    env::current_dir()
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| String::from("default"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_overrides_all_fields() {
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
        env::remove_var("VITRIOL_GEN_PORT");
        env::remove_var("VITRIOL_HERM_PORT");
        env::remove_var("VITRIOL_EMBED_PORT");
        env::remove_var("COPULA_LOG_DIR");
        env::remove_var("VITRIOL_PROJECT_ID");
        let cfg = Config::from_env();
        assert_eq!(cfg.gen_port, 8279);
        assert_eq!(cfg.hermetis_port, 8090);
        assert_eq!(cfg.embed_port, 8081);
        assert_eq!(cfg.log_dir, PathBuf::from("/tmp/opencode"));
    }
}
