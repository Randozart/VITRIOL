//! Ascensus secrets: the Gemini API key + chosen model, persisted OUTSIDE the
//! repo at `~/.vitriol/secrets` with 0600 permissions.
//!
//! Kept separate from `~/.vitriol/config` so profile save/load and config
//! exports never carry the key. The TUI writes only the masked form of the key;
//! this module never prints or logs the full value.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// INI section used by the secrets file.
const SECTION: &str = "ascensus";

/// Loaded Ascensus secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secrets {
    /// Gemini API key (empty when unset).
    pub api_key: String,
    /// Gemini model id (empty when unset).
    pub model: String,
}

impl Secrets {
    /// Empty secrets (no key, no model).
    pub fn empty() -> Self {
        Self {
            api_key: String::new(),
            model: String::new(),
        }
    }

    /// True when a non-empty API key is set.
    pub fn has_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// Read the secrets file. Missing or unreadable file -> empty secrets
    /// (never errors).
    pub fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::empty();
        };
        let mut s = Self::empty();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim();
                if k == "api_key" {
                    s.api_key = v.trim().to_string();
                } else if k == "model" {
                    s.model = v.trim().to_string();
                }
            }
        }
        s
    }

    /// Write the secrets file atomically with 0600 perms. The API key is never
    /// rendered into any string we print; only this write sees it.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = format!(
            "# vitriol-tui managed — do not commit or share\n[{SECTION}]\napi_key = {}\nmodel = {}\n",
            self.api_key, self.model
        );
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let stem = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "secrets".into());
        let tmp = dir.join(format!(".{stem}.tmp"));
        fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
        fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))
    }

    /// The masked key for display: `••••` + last 4 chars when set, else `unset`.
    /// Never reveals more than the final 4 characters.
    pub fn mask(&self) -> String {
        let k = self.api_key.trim();
        if k.is_empty() {
            "unset".into()
        } else if k.len() <= 4 {
            "••••".into()
        } else {
            format!("••••{}", &k[k.len() - 4..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vitriol_secrets_{name}.db"))
    }

    #[test]
    fn save_load_roundtrip() {
        let p = tmp_path("roundtrip");
        let _ = fs::remove_file(&p);
        let s = Secrets {
            api_key: "AIza-supersecret123".into(),
            model: "gemini-2.5-flash".into(),
        };
        s.save(&p).unwrap();
        let loaded = Secrets::load(&p);
        assert_eq!(loaded.api_key, "AIza-supersecret123");
        assert_eq!(loaded.model, "gemini-2.5-flash");
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn save_sets_0600_perms() {
        let p = tmp_path("perms");
        let _ = fs::remove_file(&p);
        Secrets {
            api_key: "k".into(),
            model: "m".into(),
        }
        .save(&p)
        .unwrap();
        let mode = fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn missing_file_is_empty() {
        let p = tmp_path("missing");
        let _ = fs::remove_file(&p);
        let s = Secrets::load(&p);
        assert!(!s.has_key());
        assert_eq!(s.mask(), "unset");
    }

    #[test]
    fn mask_never_reveals_full_key() {
        let s = Secrets {
            api_key: "AIzaABCDEF123456".into(),
            model: "m".into(),
        };
        let m = s.mask();
        assert!(!m.contains("AIza"));
        assert!(m.ends_with("3456"));
        assert_eq!(m.chars().count(), 8);
    }

    #[test]
    fn short_key_masks_fully() {
        let s = Secrets {
            api_key: "abc".into(),
            model: "m".into(),
        };
        assert_eq!(s.mask(), "••••");
    }
}
