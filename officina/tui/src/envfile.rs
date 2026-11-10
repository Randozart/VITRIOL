// Officina launch-env loader (parity with officina.mjs, 2026-11-10).
//
// The JS frontend read ~/.vitriol/officina/env before extension binding.
// The Rust TUI spawned pi directly and ignored it, so OFFICINA_ROUTE_MODE,
// OFFICINA_CADUCEUS*, OFFICINA_CACHE_MODE, OFFICINA_PRAETOR, TRIS_* and
// friends silently did not apply to the canonical frontend.
//
// Semantics match the JS loader: KEY=VALUE lines, # comments, optional single
// or double quotes, and an already-exported shell variable always wins.

use std::fs;
use std::path::PathBuf;

pub fn officina_env_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OFFICINA_ENV_FILE") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".vitriol")
            .join("officina")
            .join("env"),
    )
}

/// Parse the file into ordered KEY/VALUE pairs. Malformed lines are ignored.
pub fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let mut value = value.trim();
        if value.len() >= 2 {
            let bytes = value.as_bytes();
            let quoted = (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'');
            if quoted {
                value = &value[1..value.len() - 1];
            }
        }
        out.push((key.to_string(), value.to_string()));
    }
    out
}

/// Apply the file to the current process; existing env wins (`??=` parity).
pub fn load_officina_env() {
    let Some(path) = officina_env_path() else {
        return;
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    for (key, value) in parse_env(&text) {
        if std::env::var_os(&key).is_none() {
            std::env::set_var(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_quotes_and_ignores_junk() {
        let text =
            "# comment\n\nOFFICINA_X=1\nOFFICINA_Y = \"two words\"\nZ='three'\nbadline\n=no-key\n";
        assert_eq!(
            parse_env(text),
            vec![
                ("OFFICINA_X".to_string(), "1".to_string()),
                ("OFFICINA_Y".to_string(), "two words".to_string()),
                ("Z".to_string(), "three".to_string()),
            ]
        );
    }
}

/// Runtime permission-mode file. Kept separate from the generated
/// permissions.json so `trismegistus perms-sync` cannot clobber the mode.
pub fn permissions_mode_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OFFICINA_PERMISSIONS_MODE_FILE") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".vitriol")
            .join("officina")
            .join("permissions-mode"),
    )
}

pub fn read_permissions_mode() -> String {
    if let Some(path) = permissions_mode_path() {
        if let Ok(text) = fs::read_to_string(path) {
            let mode = text.trim().to_ascii_lowercase();
            if mode == "ask" || mode == "auto" || mode == "yolo" {
                return mode;
            }
        }
    }
    let env = std::env::var("OFFICINA_PERMISSIONS_MODE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if env == "auto" || env == "yolo" {
        env
    } else {
        "ask".to_string()
    }
}

/// Replace `key=...` in the env file (append if absent), preserving comments.
pub fn update_env_text(text: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key}=")) || trimmed.starts_with(&format!("{key} =")) {
            out.push(format!("{key}={value}"));
            found = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !found {
        out.push(format!("{key}={value}"));
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

pub fn write_permissions_mode(mode: &str) -> std::io::Result<()> {
    if let Some(path) = permissions_mode_path() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{mode}\n"))?;
    }
    if let Some(path) = officina_env_path() {
        let old = fs::read_to_string(&path).unwrap_or_default();
        let next = update_env_text(&old, "OFFICINA_PERMISSIONS_MODE", mode);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, next)?;
    }
    std::env::set_var("OFFICINA_PERMISSIONS_MODE", mode);
    Ok(())
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn env_text_replaces_and_appends() {
        let src = "# keep\nOFFICINA_X=1\nOFFICINA_PERMISSIONS_MODE=ask\n";
        let out = update_env_text(src, "OFFICINA_PERMISSIONS_MODE", "yolo");
        assert!(out.contains("# keep"));
        assert!(out.contains("OFFICINA_PERMISSIONS_MODE=yolo"));
        assert_eq!(out.matches("OFFICINA_PERMISSIONS_MODE=").count(), 1);
        let appended = update_env_text("# c\n", "A", "b");
        assert!(appended.contains("A=b"));
    }
}
