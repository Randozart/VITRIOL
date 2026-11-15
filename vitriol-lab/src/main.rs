// PROVENANCE: inspiration — none; explicit offline experiment CLI.
use serde_json::{json, Value};
use std::io::Read;
use vitriol_lab::{governor, kernel, replay, validation};

const USAGE: &str = "vitriol-lab replay|compare|governor|kernel-check|validate INPUT.json";

fn reference() -> Result<Value, String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("kernel input exceeds 1 MiB".into());
    }
    let operation: kernel::Operation = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(json!({"version":1,"output":operation.reference()?,"backend":"vitriol-lab CPU reference"}))
}

fn kernel_check(input: Value) -> Result<Value, String> {
    let mut suite: kernel::Suite = serde_json::from_value(input).map_err(|e| e.to_string())?;
    if suite.backend.program == std::path::Path::new("self") {
        if suite.backend.kind != kernel::Kind::CpuReference || suite.backend.args != ["reference"] {
            return Err("self is reserved for the CPU reference protocol".into());
        }
        suite.backend.program = std::env::current_exe().map_err(|e| e.to_string())?;
    }
    serde_json::to_value(kernel::check(&suite)?).map_err(|e| e.to_string())
}

fn load(path: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("input exceeds 16 MiB".into());
    }
    Ok(bytes)
}

fn run(args: &[String]) -> Result<Value, String> {
    if args == ["reference"] {
        return reference();
    }
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        return Ok(json!({"usage":USAGE, "hardware_verified":false}));
    }
    if args.len() != 2 {
        return Err(USAGE.into());
    }
    let bytes = load(&args[1])?;
    let input: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut report = match args[0].as_str() {
        "replay" => {
            let experiment: replay::Experiment =
                serde_json::from_value(input).map_err(|e| e.to_string())?;
            if experiment.candidates.len() != 1 {
                return Err(
                    "replay requires exactly one candidate; use compare for a sweep".into(),
                );
            }
            serde_json::to_value(replay::replay(
                &experiment.trace,
                &experiment.candidates[0],
            )?)
            .map_err(|e| e.to_string())
        }
        "compare" => serde_json::to_value(replay::compare(
            &serde_json::from_value(input).map_err(|e| e.to_string())?,
        )?)
        .map_err(|e| e.to_string()),
        "governor" => serde_json::to_value(governor::replay(
            &serde_json::from_value(input).map_err(|e| e.to_string())?,
            vitriol_lab::digest(&bytes),
        )?)
        .map_err(|e| e.to_string()),
        "kernel-check" => kernel_check(input),
        "validate" => serde_json::to_value(validation::check(
            &serde_json::from_value(input).map_err(|e| e.to_string())?,
        )?)
        .map_err(|e| e.to_string()),
        _ => Err("unknown lab command".into()),
    }?;
    report["input_sha256"] = json!(vitriol_lab::digest(&bytes));
    report["command_argv"] = json!(std::env::args().collect::<Vec<_>>());
    Ok(report)
}

fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(value) => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
            match value.get("status").and_then(Value::as_str) {
                Some("failed") => std::process::exit(1),
                Some("unavailable" | "inconclusive") => std::process::exit(2),
                _ => {}
            }
        }
        Err(error) => {
            eprintln!("{}", json!({"status":"error","error":error}));
            std::process::exit(1);
        }
    }
}
