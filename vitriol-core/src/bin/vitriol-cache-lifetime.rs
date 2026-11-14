//! Read-only native audit of declared lifecycle facts or the anonymous disk mirror.

use std::fs;
use std::path::Path;
use std::process::ExitCode;
use vitriol_core::cache_lifetime::{plan, Inventory, Plan, Snapshot};

const USAGE: &str = "usage: vitriol cache-lifetime (--disk-dir DIR | --inventory FILE) [--json]\n\
Audits snapshot lifetime using the Rust policy. Never deletes cache files.\n\
Disk scans retain anonymous entries: session ownership and reference coverage are unknown.";

/// Enumerate only regular published mirror files; a directory scan cannot supply ownership facts.
fn disk_inventory(directory: &Path) -> Result<Inventory, String> {
    let mut snapshots = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            continue;
        }
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| "cache filename is not UTF-8")?;
        if !id.starts_with("cache-") || !id.ends_with(".bin") {
            continue;
        }
        snapshots.push(Snapshot {
            id,
            incarnation: 0,
            model_key: "unknown".into(),
            layout_key: "unknown".into(),
            bytes: metadata.len(),
            durable: false,
            owners: None,
            pins: vec![],
            readers: 0,
            writers: 0,
        });
    }
    snapshots.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Inventory {
        version: 1,
        epoch: 0,
        reference_coverage_complete: false,
        snapshots,
        retirements: vec![],
    })
}

/// Load operator-supplied lifecycle facts without inventing missing protection metadata.
fn load_inventory(mode: &str, path: &str) -> Result<Inventory, String> {
    if mode == "--disk-dir" {
        return disk_inventory(Path::new(path));
    }
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&contents).map_err(|error| format!("invalid cache inventory: {error}"))
}

/// Report conditional eligibility and the physical-execution boundary together.
fn render(plan: &Plan, complete: bool) -> String {
    let mut output = format!("Cache lifetime audit: {} snapshots, {} bytes\nConditional retirement eligibility: {} bytes\nReference coverage complete: {}\nPhysical deletion authorized: {}\n",
        plan.decisions.len(), plan.total_bytes, plan.eligible_bytes, complete, plan.deletion_authorized);
    for decision in &plan.decisions {
        let action = if decision.eligible {
            "eligible"
        } else {
            "retain"
        };
        output.push_str(&format!(
            "{action} {}: {:?}\n",
            decision.id, decision.reason
        ));
    }
    output
}

/// Accept exactly one inventory source; an accidental extra flag must never select another mode.
fn run(arguments: &[String]) -> Result<String, String> {
    if arguments == ["--help"] || arguments == ["-h"] {
        return Ok(USAGE.into());
    }
    let json = arguments.last().is_some_and(|arg| arg == "--json");
    let source = if json {
        &arguments[..arguments.len() - 1]
    } else {
        arguments
    };
    let [mode, path] = source else {
        return Err(USAGE.into());
    };
    if mode != "--disk-dir" && mode != "--inventory" {
        return Err(USAGE.into());
    }
    let inventory = load_inventory(mode, path)?;
    let result = plan(&inventory)?;
    if json {
        return serde_json::to_string_pretty(&result).map_err(|error| error.to_string());
    }
    Ok(render(&result, inventory.reference_coverage_complete))
}

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
