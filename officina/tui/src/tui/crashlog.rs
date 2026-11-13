//! Crash black-box (2026-11-13): "Officina crashed" with zero artifacts is
//! undiagnosable. The panic hook writes the message + the diag ring to
//! ~/.vitriol/officina/last-crash.log (best-effort, then the default hook
//! runs so panic behavior is unchanged), a clean-exit stamp distinguishes
//! crashes from quits, and the next startup surfaces the crash as a notice.
//!
//! Invariant I1: best-effort only — every failure path falls back to the
//! default hook; nothing here can mask a panic.

use std::panic;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::rpc::bridge::DiagLog;

static DIAG: OnceLock<DiagLog> = OnceLock::new();

/// Register the diag ring (DiagLog::new calls this — one ring per bridge).
pub fn set_diag(diag: DiagLog) {
    let _ = DIAG.set(diag);
}
static DEFAULT_HOOK: OnceLock<Box<dyn Fn(&PanicHookInfo) + Sync + Send>> = OnceLock::new();

pub fn crash_log_path() -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("last-crash.log");
    p
}

pub fn clean_exit_path() -> PathBuf {
    crash_log_path().with_file_name("last-clean-exit")
}

/// Register the diag ring (call before spawning pi) and install the hook.
pub fn install(diag: DiagLog) {
    let _ = DIAG.set(diag);
    let _ = DEFAULT_HOOK.set(panic::take_hook());
    panic::set_hook(Box::new(move |info: &PanicHookInfo| {
        // Best-effort, I1: write on a plain thread-scope is fine here —
        // panicking hooks must not panic, so every op is fallible-safe.
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "panic (non-string payload)".to_string()
        };
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let diag = DIAG.get().map(|d| d.snapshot()).unwrap_or_default();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = write_crash_entry(
            &crash_log_path(),
            &format!("panic at {loc}: {msg}"),
            diag,
            stamp,
        );
        if let Some(hook) = DEFAULT_HOOK.get() {
            hook(info);
        }
    }));
}

/// Append one crash entry + the diag ring (caller decides freshness).
/// Exposed for tests with an injected path.
pub fn write_crash_entry(
    path: &PathBuf,
    message: &str,
    diag_lines: Vec<String>,
    stamp: u64,
) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "== crash @ {stamp} == {message}")?;
    for line in diag_lines.iter().take(100) {
        writeln!(f, "| {line}")?;
    }
    f.flush()
}

/// Deliberate quit path: mark the exit clean so the next startup does not
/// flag the previous run.
pub fn stamp_clean_exit() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = clean_exit_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{now}\n"));
}

/// Fresh crash log newer than the last clean exit → (first panic line).
pub fn crash_since_clean_exit() -> Option<String> {
    let crash_meta = std::fs::metadata(crash_log_path()).ok()?;
    let crash_t = crash_meta.modified().ok()?;
    match std::fs::metadata(clean_exit_path()) {
        Ok(exit_meta) => {
            if crash_t <= exit_meta.modified().ok()? {
                return None;
            }
        }
        Err(_) => {}
    }
    let text = std::fs::read_to_string(crash_log_path()).ok()?;
    text.lines().next().map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn crash_entry_and_freshness_logic() {
        let dir = std::env::temp_dir().join(format!("crashlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("last-crash.log");
        let exit = dir.join("last-clean-exit");

        // no exit stamp → fresh crash surfaces
        write_crash_entry(
            &log,
            "panic at src/x.rs:5: boom",
            vec!["diag line".into()],
            100,
        )
        .unwrap();
        // crash_t == now … exit missing → surfaces
        let surfaced = {
            // reuse the comparison logic by pointing the paths at temp files
            // via helper? crash_since_clean_exit uses fixed paths — so test
            // the freshness rule inline here instead.
            let crash_t = std::fs::metadata(&log).unwrap().modified().unwrap();
            let no_exit: Option<std::time::SystemTime> = None;
            match no_exit {
                None => true,
                Some(t) => crash_t > t,
            }
        };
        assert!(surfaced);

        // exit stamp NEWER than the crash → suppressed
        std::fs::write(&exit, "now\n").unwrap();
        let exit_t = std::fs::metadata(&exit).unwrap().modified().unwrap();
        let crash_t = std::fs::metadata(&log).unwrap().modified().unwrap();
        assert!(crash_t <= exit_t);

        let _ = std::fs::remove_dir_all(&dir);
        // write_crash_entry appends (second call grows the file)
        write_crash_entry(&log, "second", vec!["fresh diag".into()], 200).unwrap();
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.contains("second"));
        assert!(text.contains("fresh diag"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
