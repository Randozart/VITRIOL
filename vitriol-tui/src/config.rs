//! Paths and endpoint configuration for the operations console.

use std::env;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Config {
    pub gen_port: Option<u16>,
    pub log_dir: PathBuf,
    pub repo_root: PathBuf,
    pub home_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let repo_root = env::var_os("VITRIOL_REPO")
            .map(PathBuf::from)
            .or_else(|| {
                cwd.ancestors()
                    .find(|dir| dir.join("scripts/vitriol").is_file())
                    .map(PathBuf::from)
            })
            .unwrap_or_else(|| cwd.clone());
        Self {
            gen_port: env::var("VITRIOL_GEN_PORT")
                .ok()
                .and_then(|value| value.parse().ok()),
            log_dir: env::var_os("COPULA_LOG_DIR")
                .map_or_else(|| "/tmp/opencode".into(), PathBuf::from),
            home_dir: env::var_os("HOME").map_or(cwd, PathBuf::from),
            repo_root,
        }
    }

    pub fn gen_log(&self) -> PathBuf {
        self.log_dir.join("vitriol_gen.log")
    }

    pub fn vitriol_cli(&self) -> PathBuf {
        self.repo_root.join("scripts/vitriol")
    }

    pub fn active_config(&self) -> PathBuf {
        self.home_dir.join(".vitriol/config")
    }

    pub fn bundled_profiles_dir(&self) -> PathBuf {
        self.repo_root.join("profiles")
    }

    pub fn installed_profiles_dir(&self) -> PathBuf {
        self.home_dir.join(".vitriol/profiles")
    }
}
