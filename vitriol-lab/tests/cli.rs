use serde_json::Value;
use std::process::Command;

fn run(command: &str, fixture: &str) -> (i32, Value) {
    let path = format!("{}/examples/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let result = Command::new(env!("CARGO_BIN_EXE_vitriol-lab"))
        .args([command, &path])
        .output()
        .unwrap();
    let value = serde_json::from_slice(&result.stdout).unwrap_or_else(|_| {
        panic!(
            "no JSON response: {}",
            String::from_utf8_lossy(&result.stderr)
        )
    });
    (result.status.code().unwrap(), value)
}

#[test]
fn examples_are_executable_and_bind_input_bytes_and_actual_arguments() {
    let (code, value) = run("compare", "sessions.json");
    assert_eq!(code, 0);
    assert_eq!(value["input_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(value["command_argv"][1], "compare");
    assert_eq!(value["hardware_verified"], false);
    let (code, value) = run("kernel-check", "kernel-reference.json");
    assert_eq!(code, 0);
    assert_eq!(value["cases"].as_array().unwrap().len(), 3);
    assert_eq!(value["status"], "passed");
    assert_eq!(value["gpu_timing_available"], false);
    let (code, value) = run("validate", "calibration.json");
    assert_eq!(code, 0);
    assert_eq!(value["evidence"], "synthetic_calibration_fixture");
}

#[test]
fn malformed_input_and_unknown_commands_have_nonzero_exit() {
    let result = Command::new(env!("CARGO_BIN_EXE_vitriol-lab"))
        .args(["unknown", "/nonexistent/lab-fixture"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stderr).unwrap()["status"],
        "error"
    );
}
