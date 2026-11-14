use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const FIXTURE: &str = include_str!("../testdata/cache-lifetime-inventory.json");

struct Directory(PathBuf);

impl Directory {
    /// Create a test-owned directory; never reuse or delete a pre-existing path.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "vitriol-lifetime-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn legacy_disk_audit_retains_files_and_ignores_temporary_or_nonregular_entries() {
    let dir = Directory::new();
    fs::write(dir.0.join("cache-a.bin"), [1u8; 7]).unwrap();
    fs::write(dir.0.join("cache-b.bin"), [2u8; 5]).unwrap();
    fs::write(dir.0.join("cache-inflight.bin.tmp"), [3u8; 50]).unwrap();
    fs::write(dir.0.join("checkpoint.bin"), [4u8; 20]).unwrap();
    fs::create_dir(dir.0.join("cache-folder.bin")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.0.join("cache-a.bin"), dir.0.join("cache-link.bin")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vitriol-cache-lifetime"))
        .arg("--disk-dir")
        .arg(&dir.0)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let audit: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(audit["totalBytes"], 12);
    assert_eq!(audit["eligibleBytes"], 0);
    assert_eq!(audit["deletionAuthorized"], false);
    let decisions = audit["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 2);
    assert!(decisions
        .iter()
        .all(|d| d["reason"] == "unknown-ownership" && d["eligible"] == false));
    assert_eq!(fs::read(dir.0.join("cache-a.bin")).unwrap(), [1u8; 7]);
    assert_eq!(fs::read(dir.0.join("cache-b.bin")).unwrap(), [2u8; 5]);
    assert!(dir.0.join("cache-inflight.bin.tmp").exists());
}

#[test]
fn cli_uses_the_rust_policy_for_declared_inventory_and_rejects_delete_flags() {
    let dir = Directory::new();
    let path = dir.0.join("inventory.json");
    fs::write(&path, FIXTURE).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vitriol-cache-lifetime"))
        .arg("--inventory")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["eligibleBytes"], 100);
    assert_eq!(result["deletionAuthorized"], false);
    let refused = Command::new(env!("CARGO_BIN_EXE_vitriol-cache-lifetime"))
        .arg("--inventory")
        .arg(&path)
        .arg("--delete")
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), FIXTURE);
}

#[test]
fn invalid_inventory_fails_without_a_retirement_result() {
    let dir = Directory::new();
    let path = dir.0.join("bad.json");
    fs::write(&path, "{\"version\":999}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vitriol-cache-lifetime"))
        .arg("--inventory")
        .arg(path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid cache inventory"));
}

#[test]
fn stdio_rpc_preserves_existing_methods_and_fails_closed_for_bad_lifetime_facts() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vitriol-core-rpc"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let inventory: Value = serde_json::from_str(FIXTURE).unwrap();
    let requests = [
        json!({"id":1,"method":"ping"}),
        json!({"id":2,"method":"cache_retirement_plan","params":inventory}),
        json!({"id":3,"method":"cache_retirement_plan","params":{}}),
        json!({"id":4,"method":"admit","params":{"promptTokens":10,"lanes":[]}}),
    ];
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["result"]["ok"], true);
    assert_eq!(responses[1]["id"], 2);
    assert_eq!(responses[1]["result"]["eligibleBytes"], 100);
    assert_eq!(responses[1]["result"]["deletionAuthorized"], false);
    assert!(responses[2].get("result").is_none());
    assert!(responses[2]["error"]
        .as_str()
        .unwrap()
        .contains("invalid cache inventory"));
    assert_eq!(responses[3]["result"]["decisions"], json!({}));
}
