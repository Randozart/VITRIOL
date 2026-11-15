// PROVENANCE: inspiration — none; VITRIOL-original held-out prediction error and ordering checks.
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub name: String,
    pub predicted_us: f64,
    pub measured_us: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub version: u32,
    pub measurement_origin: crate::model::Origin,
    pub held_out: bool,
    pub engine_fingerprint: String,
    pub hardware_fingerprint: String,
    pub trace_sha256: String,
    pub baseline: String,
    pub max_relative_error: f64,
    pub minimum_effect_fraction: f64,
    pub measurements: Vec<Measurement>,
}

#[derive(Serialize)]
pub struct Comparison {
    pub candidate: String,
    pub relative_error: f64,
    pub ordering: &'static str,
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub evidence: &'static str,
    pub status: &'static str,
    pub hardware_verified: bool,
    pub engine_fingerprint: String,
    pub hardware_fingerprint: String,
    pub trace_sha256: String,
    pub worst_relative_error: f64,
    pub resolved_comparisons: usize,
    pub comparisons: Vec<Comparison>,
}

fn validate(input: &Calibration) -> Result<(), String> {
    if input.version != 1 || input.measurements.len() < 2 || input.measurements.len() > 10_000 {
        return Err("calibration needs version1 and 2..10000 measurements".into());
    }
    crate::label(&input.engine_fingerprint)?;
    crate::label(&input.hardware_fingerprint)?;
    if input.trace_sha256.len() != 64 || !input.trace_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("trace_sha256 must be a SHA256 hex string".into());
    }
    let bounds = [input.max_relative_error, input.minimum_effect_fraction];
    if bounds
        .iter()
        .any(|v| !v.is_finite() || !(0.0..1.0).contains(v))
    {
        return Err("calibration fractions must be finite and within [0,1)".into());
    }
    if !input.held_out {
        return Err("calibration requires held-out measurements".into());
    }
    validate_measurements(&input.measurements)
}

fn validate_measurements(measurements: &[Measurement]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for row in measurements {
        crate::label(&row.name)?;
        if !names.insert(&row.name)
            || [row.predicted_us, row.measured_us]
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 1e15)
        {
            return Err(
                "measurement names must be unique and times finite, positive and <=1e15 us".into(),
            );
        }
    }
    Ok(())
}

fn ordering(row: &Measurement, baseline: &Measurement, input: &Calibration) -> &'static str {
    let measured = baseline.measured_us - row.measured_us;
    let uncertainty = input.max_relative_error * (baseline.measured_us + row.measured_us);
    if measured.abs() <= uncertainty.max(input.minimum_effect_fraction * baseline.measured_us) {
        return "unresolved";
    }
    if measured * (baseline.predicted_us - row.predicted_us) > 0.0 {
        "agrees"
    } else {
        "disagrees"
    }
}

fn verdict(worst: f64, maximum: f64, comparisons: &[Comparison]) -> (&'static str, usize) {
    let resolved = comparisons
        .iter()
        .filter(|c| c.ordering != "unresolved")
        .count();
    if worst > maximum || comparisons.iter().any(|c| c.ordering == "disagrees") {
        return ("failed", resolved);
    }
    if resolved == 0 {
        return ("inconclusive", 0);
    }
    ("passed", resolved)
}

pub fn check(input: &Calibration) -> Result<Report, String> {
    validate(input)?;
    let baseline = input
        .measurements
        .iter()
        .find(|m| m.name == input.baseline)
        .ok_or("unknown calibration baseline")?;
    let mut comparisons = Vec::new();
    let mut worst: f64 = 0.0;
    for row in &input.measurements {
        let relative_error = (row.predicted_us - row.measured_us).abs() / row.measured_us;
        if !relative_error.is_finite() {
            return Err("relative error overflow".into());
        }
        worst = worst.max(relative_error);
        if row.name == input.baseline {
            continue;
        }
        comparisons.push(Comparison {
            candidate: row.name.clone(),
            relative_error,
            ordering: ordering(row, baseline, input),
        });
    }
    let (status, resolved) = verdict(worst, input.max_relative_error, &comparisons);
    Ok(Report {
        version: 1,
        evidence: if matches!(input.measurement_origin, crate::model::Origin::Synthetic) {
            "synthetic_calibration_fixture"
        } else {
            "user_supplied_held_out_measurements"
        },
        status,
        hardware_verified: false,
        engine_fingerprint: input.engine_fingerprint.clone(),
        hardware_fingerprint: input.hardware_fingerprint.clone(),
        trace_sha256: input.trace_sha256.clone(),
        worst_relative_error: worst,
        resolved_comparisons: resolved,
        comparisons,
    })
}
