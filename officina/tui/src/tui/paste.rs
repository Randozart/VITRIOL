// Clipboard image paste (codex-qol stage 9). Pure helpers live here; the
// keybinding + probe glue stays in mod.rs. Invariant I5: only image/*
// clipboard data is attached, caps are enforced, and every failure is a
// notice — never a crash or a silent drop of the composer draft.

use crate::rpc::protocol::ImageContent;
use base64::Engine;
use std::io;
use std::path::{Path, PathBuf};

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMAGES: usize = 4;

/// Magic-byte validation — never trust the clipboard MIME label alone.
pub fn magic_ok(bytes: &[u8], mime: &str) -> bool {
    match mime {
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        _ => false,
    }
}

/// Clipboard probe candidates in try order (Wayland first, X11 fallback).
pub fn clipboard_candidates(wayland: bool) -> Vec<(String, Vec<String>, String)> {
    if wayland {
        vec![
            (
                "wl-paste".to_string(),
                vec!["--no-newline".into(), "--type".into(), "image/png".into()],
                "image/png".to_string(),
            ),
            (
                "wl-paste".to_string(),
                vec!["--no-newline".into(), "--type".into(), "image/jpeg".into()],
                "image/jpeg".to_string(),
            ),
        ]
    } else {
        vec![
            (
                "xclip".to_string(),
                vec![
                    "-selection".into(),
                    "clipboard".into(),
                    "-t".into(),
                    "image/png".into(),
                    "-o".into(),
                ],
                "image/png".to_string(),
            ),
            (
                "xclip".to_string(),
                vec![
                    "-selection".into(),
                    "clipboard".into(),
                    "-t".into(),
                    "image/jpeg".into(),
                    "-o".into(),
                ],
                "image/jpeg".to_string(),
            ),
        ]
    }
}

/// Save clipboard bytes to a temp file; returns the path.
pub fn save_temp(bytes: &[u8], mime: &str) -> io::Result<PathBuf> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("image exceeds {} MiB cap", MAX_IMAGE_BYTES / (1024 * 1024)),
        ));
    }
    let ext = if mime == "image/jpeg" { "jpg" } else { "png" };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("officina-paste-{nanos}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Read a pending image back as an RPC ImageContent (base64 data).
pub fn encode_image(path: &Path, mime: &str) -> io::Result<ImageContent> {
    let bytes = std::fs::read(path)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "image exceeds cap",
        ));
    }
    Ok(ImageContent {
        content_type: "image".to_string(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        mime_type: mime.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];
    const JPEG_MAGIC: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];

    #[test]
    fn magic_bytes_gate_mime_labels() {
        assert!(magic_ok(PNG_MAGIC, "image/png"));
        assert!(magic_ok(JPEG_MAGIC, "image/jpeg"));
        // lying clipboard label is rejected
        assert!(!magic_ok(PNG_MAGIC, "image/jpeg"));
        assert!(!magic_ok(b"hello text", "image/png"));
        assert!(!magic_ok(&[], "image/png"));
    }

    #[test]
    fn save_temp_enforces_the_cap_and_round_trips() {
        assert!(save_temp(&vec![0u8; MAX_IMAGE_BYTES + 1], "image/png").is_err());
        let path = save_temp(PNG_MAGIC, "image/png").expect("saved");
        assert!(path.extension().unwrap() == "png");
        let encoded = encode_image(&path, "image/png").expect("encoded");
        assert_eq!(encoded.content_type, "image");
        assert_eq!(encoded.mime_type, "image/png");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded.data)
            .expect("base64 round-trip");
        assert_eq!(decoded, PNG_MAGIC);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn candidates_prefer_wayland_when_present() {
        let wl = clipboard_candidates(true);
        assert_eq!(wl[0].0, "wl-paste");
        assert_eq!(wl[0].2, "image/png");
        let x = clipboard_candidates(false);
        assert_eq!(x[0].0, "xclip");
        assert_eq!(x.len(), 2);
    }
}
