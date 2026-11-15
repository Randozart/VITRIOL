// PROVENANCE: inspiration — none; explicit offline experiment CLI.
use serde_json::{json, Value};
use std::io::Read;
use vitriol_lab::{governor, replay};

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
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        return Ok(
            json!({"usage":"vitriol-lab replay|compare|governor INPUT.json", "hardware_verified":false}),
        );
    }
    if args.len() != 2 {
        return Err("usage: vitriol-lab replay|compare|governor INPUT.json".into());
    }
    let bytes = load(&args[1])?;
    let input: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    match args[0].as_str() {
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
        _ => Err("unknown lab command".into()),
    }
}

fn main() {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(error) => {
            eprintln!("{}", json!({"status":"error","error":error}));
            std::process::exit(1);
        }
    }
}
