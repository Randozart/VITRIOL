// PROVENANCE: inspiration — none; VITRIOL-original independent numerical oracle and adapter protocol.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Matvec {
        rows: usize,
        cols: usize,
        matrix: Vec<f64>,
        vector: Vec<f64>,
    },
    Dot {
        left: Vec<f64>,
        right: Vec<f64>,
    },
    Softmax {
        values: Vec<f64>,
    },
}

fn finite(values: &[f64]) -> Result<(), String> {
    if values.is_empty() || values.len() > 65_536 || values.iter().any(|v| !v.is_finite()) {
        return Err("kernel arrays must contain 1..65536 finite numbers".into());
    }
    Ok(())
}

impl Operation {
    pub fn reference(&self) -> Result<Vec<f64>, String> {
        let output = match self {
            Self::Matvec {
                rows,
                cols,
                matrix,
                vector,
            } => matvec(*rows, *cols, matrix, vector)?,
            Self::Dot { left, right } => {
                finite(left)?;
                finite(right)?;
                if left.len() != right.len() {
                    return Err("dot vector lengths differ".into());
                }
                vec![left.iter().zip(right).map(|(a, b)| a * b).sum()]
            }
            Self::Softmax { values } => {
                finite(values)?;
                let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let exps: Vec<_> = values.iter().map(|v| (v - maximum).exp()).collect();
                let sum: f64 = exps.iter().sum();
                exps.iter().map(|v| v / sum).collect()
            }
        };
        finite(&output)?;
        Ok(output)
    }
}

fn matvec(rows: usize, cols: usize, matrix: &[f64], vector: &[f64]) -> Result<Vec<f64>, String> {
    finite(matrix)?;
    finite(vector)?;
    if rows == 0
        || rows > 4096
        || cols == 0
        || rows.checked_mul(cols) != Some(matrix.len())
        || cols != vector.len()
    {
        return Err("matvec shape mismatch or row bound exceeded".into());
    }
    Ok(matrix
        .chunks_exact(cols)
        .map(|row| row.iter().zip(vector).map(|(a, b)| a * b).sum())
        .collect())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    CpuReference,
    FunctionalEmulation,
    DeclaredHardware,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub kind: Kind,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub artifacts: Vec<PathBuf>,
    pub timeout_ms: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub input: Operation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub version: u32,
    pub backend: Backend,
    pub abs_tolerance: f64,
    pub rel_tolerance: f64,
    pub cases: Vec<Case>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub output: Vec<f64>,
    pub backend: String,
}

#[derive(Serialize)]
pub struct ResultRow {
    pub id: String,
    pub status: &'static str,
    pub message: String,
    pub input_sha256: String,
    pub stdout_sha256: Option<String>,
    pub backend: Option<String>,
    pub max_abs_error: Option<f64>,
    pub runner_wall_us: u64,
    pub stderr: String,
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub evidence: &'static str,
    pub status: &'static str,
    pub hardware_verified: bool,
    pub gpu_timing_available: bool,
    pub backend: Backend,
    pub artifacts_sha256: BTreeMap<PathBuf, String>,
    pub artifact_integrity: &'static str,
    pub cases: Vec<ResultRow>,
}

fn validate(suite: &Suite) -> Result<(), String> {
    if suite.version != 1 || suite.cases.is_empty() || suite.cases.len() > 64 {
        return Err("kernel suite requires version1 and 1..64 cases".into());
    }
    validate_backend(&suite.backend)?;
    let tolerance = [suite.abs_tolerance, suite.rel_tolerance];
    if tolerance
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0 || *v > 1.0)
    {
        return Err("tolerances must be finite and within 0..1".into());
    }
    validate_cases(&suite.cases)
}

fn validate_backend(backend: &Backend) -> Result<(), String> {
    if !backend.program.is_absolute() {
        return Err("backend program must be an absolute executable path".into());
    }
    if backend.timeout_ms == 0
        || backend.timeout_ms > 60_000
        || backend.args.len() > 64
        || backend.artifacts.len() > 64
    {
        return Err("backend bounds exceeded (timeout 1..60000 ms)".into());
    }
    Ok(())
}

fn validate_cases(cases: &[Case]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for case in cases {
        crate::label(&case.id)?;
        if !ids.insert(&case.id) {
            return Err("kernel case IDs must be unique".into());
        }
        case.input.reference()?;
    }
    Ok(())
}

fn artifacts(backend: &Backend) -> Result<BTreeMap<PathBuf, String>, String> {
    let paths = std::iter::once(&backend.program).chain(&backend.artifacts);
    paths
        .map(|p| Ok((p.clone(), crate::file_digest(p)?)))
        .collect()
}

fn compare(expected: &[f64], actual: &[f64], suite: &Suite) -> Result<f64, String> {
    if expected.len() != actual.len() {
        return Err("backend output length mismatch".into());
    }
    finite(actual)?;
    let mut maximum: f64 = 0.0;
    for (i, (reference, result)) in expected.iter().zip(actual).enumerate() {
        let error = (reference - result).abs();
        let tolerance = suite.abs_tolerance + suite.rel_tolerance * reference.abs();
        if !error.is_finite() || error > tolerance {
            return Err(format!(
                "element {i}: absolute error {error} exceeds tolerance {tolerance}"
            ));
        }
        maximum = maximum.max(error);
    }
    Ok(maximum)
}

fn checked_result(case: &Case, suite: &Suite) -> Result<ResultRow, String> {
    let input = serde_json::to_vec(&case.input).map_err(|e| e.to_string())?;
    if input.len() > 1_048_576 {
        return Err("kernel protocol input exceeds 1 MiB".into());
    }
    let run = crate::process::execute(&suite.backend, &input)?;
    let mut result = ResultRow {
        id: case.id.clone(),
        status: run.status,
        message: run.message,
        input_sha256: crate::digest(&input),
        stdout_sha256: Some(crate::digest(&run.stdout)),
        backend: None,
        max_abs_error: None,
        runner_wall_us: run.wall_us,
        stderr: run.stderr,
    };
    if result.status != "completed" {
        return Ok(result);
    }
    let response: Response = match serde_json::from_slice(&run.stdout) {
        Ok(value) => value,
        Err(error) => {
            result.status = "failed";
            result.message = format!("invalid backend response: {error}");
            return Ok(result);
        }
    };
    if response.version != 1 {
        result.status = "failed";
        result.message = "unsupported response version".into();
        return Ok(result);
    }
    result.backend = Some(response.backend);
    match compare(&case.input.reference()?, &response.output, suite) {
        Ok(error) => {
            result.status = "passed";
            result.message = "numerical tolerance satisfied".into();
            result.max_abs_error = Some(error);
        }
        Err(error) => {
            result.status = "failed";
            result.message = error;
        }
    }
    Ok(result)
}

fn unavailable(case: &Case, message: &str) -> Result<ResultRow, String> {
    Ok(ResultRow {
        id: case.id.clone(),
        status: "unavailable",
        message: message.into(),
        input_sha256: crate::fingerprint(&case.input)?,
        stdout_sha256: None,
        backend: None,
        max_abs_error: None,
        runner_wall_us: 0,
        stderr: String::new(),
    })
}

pub fn check(suite: &Suite) -> Result<Report, String> {
    validate(suite)?;
    let hashes = artifacts(&suite.backend);
    let mut results = Vec::new();
    for case in &suite.cases {
        let result = match &hashes {
            Ok(_) => checked_result(case, suite)?,
            Err(error) => unavailable(case, error)?,
        };
        results.push(result);
    }
    let mut status = if results.iter().all(|r| r.status == "passed") {
        "passed"
    } else if results.iter().all(|r| r.status == "unavailable") {
        "unavailable"
    } else {
        "failed"
    };
    let integrity = match &hashes {
        Ok(before) if artifacts(&suite.backend).as_ref() == Ok(before) => "stable",
        Ok(_) => {
            status = "failed";
            "changed"
        }
        Err(_) => "unavailable",
    };
    Ok(Report {
        version: 1,
        evidence: "numerical_correctness_only",
        status,
        hardware_verified: false,
        gpu_timing_available: false,
        backend: suite.backend.clone(),
        artifacts_sha256: hashes.unwrap_or_default(),
        artifact_integrity: integrity,
        cases: results,
    })
}
