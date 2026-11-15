use std::{collections::BTreeMap, path::PathBuf};
use vitriol_lab::kernel::{self, Backend, Case, Kind, Operation, Suite};

fn dot() -> Operation {
    Operation::Dot {
        left: vec![1.0, 2.0],
        right: vec![3.0, 4.0],
    }
}
fn suite(program: PathBuf, args: Vec<String>) -> Suite {
    Suite {
        version: 1,
        backend: Backend {
            kind: Kind::CpuReference,
            program,
            args,
            env: BTreeMap::new(),
            artifacts: vec![],
            timeout_ms: 1000,
        },
        abs_tolerance: 1e-10,
        rel_tolerance: 1e-10,
        cases: vec![Case {
            id: "dot".into(),
            input: dot(),
        }],
    }
}
fn shell(script: &str) -> Suite {
    suite(PathBuf::from("/bin/sh"), vec!["-c".into(), script.into()])
}

#[test]
fn independent_oracle_has_known_answers_and_stable_softmax() {
    let matvec = Operation::Matvec {
        rows: 2,
        cols: 2,
        matrix: vec![1.0, 2.0, 3.0, 4.0],
        vector: vec![5.0, 6.0],
    };
    assert_eq!(matvec.reference().unwrap(), vec![17.0, 39.0]);
    assert_eq!(dot().reference().unwrap(), vec![11.0]);
    let softmax = Operation::Softmax {
        values: vec![1000.0, 1000.0],
    };
    assert_eq!(softmax.reference().unwrap(), vec![0.5, 0.5]);
}

#[test]
fn invalid_shapes_and_nonfinite_or_overflowing_values_fail() {
    assert!(Operation::Matvec {
        rows: 2,
        cols: 2,
        matrix: vec![1.0],
        vector: vec![1.0, 2.0]
    }
    .reference()
    .is_err());
    assert!(Operation::Dot {
        left: vec![f64::NAN],
        right: vec![1.0]
    }
    .reference()
    .is_err());
    assert!(Operation::Dot {
        left: vec![f64::MAX],
        right: vec![f64::MAX]
    }
    .reference()
    .is_err());
}

#[test]
fn cpu_reference_protocol_passes_without_claiming_gpu_evidence() {
    let input = suite(
        PathBuf::from(env!("CARGO_BIN_EXE_vitriol-lab")),
        vec!["reference".into()],
    );
    let report = kernel::check(&input).unwrap();
    assert_eq!(report.status, "passed");
    assert!(!report.hardware_verified);
    assert!(!report.gpu_timing_available);
    assert!(!report.artifacts_sha256.is_empty());
}

#[test]
fn wrong_results_and_malformed_or_nonfinite_responses_fail() {
    let wrong = shell("printf '%s' '{\"version\":1,\"output\":[12],\"backend\":\"wrong\"}'");
    assert_eq!(kernel::check(&wrong).unwrap().status, "failed");
    let malformed = shell("printf '%s' 'not JSON'");
    assert_eq!(kernel::check(&malformed).unwrap().status, "failed");
    let nan = shell("printf '%s' '{\"version\":1,\"output\":[NaN],\"backend\":\"wrong\"}'");
    assert_eq!(kernel::check(&nan).unwrap().status, "failed");
}

#[test]
fn optional_backend_unavailability_is_not_a_pass() {
    let missing = suite(PathBuf::from("/nonexistent/vitriol-lab-backend"), vec![]);
    assert_eq!(kernel::check(&missing).unwrap().status, "unavailable");
    assert_eq!(
        kernel::check(&suite(PathBuf::from("/dev/zero"), vec![]))
            .unwrap()
            .status,
        "unavailable"
    );
    assert_eq!(
        kernel::check(&shell("exit 77")).unwrap().status,
        "unavailable"
    );
    assert_eq!(kernel::check(&shell("exit 3")).unwrap().status, "failed");
}

#[test]
fn timeout_kills_the_child_process_group_without_waiting_for_its_sleep() {
    let mut input = shell("sleep 20 & printf '%s' \"$!\" >&2; wait");
    input.backend.timeout_ms = 40;
    let began = std::time::Instant::now();
    let report = kernel::check(&input).unwrap();
    assert_eq!(report.cases[0].status, "timed_out");
    assert!(began.elapsed() < std::time::Duration::from_secs(2));
    let pid: u32 = report.cases[0].stderr.parse().unwrap();
    if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
        assert!(
            status
                .lines()
                .any(|s| s.starts_with("State:") && s.contains('Z')),
            "grandchild still executing: {status}"
        );
    }
}

#[test]
fn output_flood_is_bounded_and_rejected() {
    let mut input = shell("while :; do printf '%01000d' 0; done");
    input.backend.timeout_ms = 500;
    let report = kernel::check(&input).unwrap();
    assert_ne!(report.status, "passed");
    assert!(report.cases[0].stderr.len() <= 8192);
}

#[test]
fn artifact_mutation_invalidates_otherwise_correct_results() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("vitriol-lab-hash-{}-{stamp}", std::process::id()));
    std::fs::write(&path, b"before").unwrap();
    let mut input=shell("printf changed > \"$VITRIOL_LAB_FIXTURE\"; printf '%s' '{\"version\":1,\"output\":[11],\"backend\":\"fixture\"}'");
    input.backend.artifacts.push(path.clone());
    input
        .backend
        .env
        .insert("VITRIOL_LAB_FIXTURE".into(), path.to_str().unwrap().into());
    let report = kernel::check(&input).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(report.cases[0].status, "passed");
    assert_eq!(report.status, "failed");
}

#[test]
fn kernel_suite_requires_real_cases_and_bounded_tolerances() {
    let mut input = shell("exit 0");
    input.cases.clear();
    assert!(kernel::check(&input).is_err());
    let mut input = shell("exit 0");
    input.rel_tolerance = f64::INFINITY;
    assert!(kernel::check(&input).is_err());
}
