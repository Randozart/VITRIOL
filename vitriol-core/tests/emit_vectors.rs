//! Golden vectors for TS<->Rust parity (plan gate I2).
//!
//! `VITRIOL_WRITE_VECTORS=1 cargo test --test emit_vectors` regenerates
//! `testdata/vectors.json`; the normal run asserts the committed file equals
//! the freshly computed vectors, so drift fails CI.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use vitriol_core::scheduler::{
    is_exhausted, probe_backoff_ms, rate_allow, select_probe_candidate, ProbeFact, ProbeLane,
    ProbeWorld, RateBucket, DEFAULT_EXHAUSTED_HORIZON_MS, DEFAULT_VERIFIED_TTL_MS,
};
use vitriol_core::{admit, Admission, LaneState};

const NOW: u64 = 1_790_000_000_000;

fn lane(id: &str, owner: &str) -> ProbeLane {
    ProbeLane { id: id.into(), owner: owner.into() }
}

fn fact(kind: &str, ok_at: Option<u64>, until: Option<u64>, next: u64, fails: u64) -> ProbeFact {
    ProbeFact { kind: kind.into(), ok_at, until, latency_ms: None, fail_count: fails, next_probe_at: next }
}

fn world_json(probes: &[(&str, ProbeFact)], invalid: &[&str], paused: &[&str], cooling: &[(&str, u64)]) -> Value {
    let probes: serde_json::Map<String, Value> = probes
        .iter()
        .map(|(id, f)| {
            (
                (*id).to_string(),
                json!({
                    "kind": f.kind,
                    "okAt": f.ok_at,
                    "until": f.until,
                    "nextProbeAt": f.next_probe_at,
                    "failCount": f.fail_count,
                }),
            )
        })
        .collect();
    let cooling: serde_json::Map<String, Value> =
        cooling.iter().map(|(id, until)| ((*id).to_string(), json!(until))).collect();
    json!({
        "probes": probes,
        "invalid": invalid,
        "pausedOwners": paused,
        "coolingUntil": cooling,
    })
}

fn build_world(value: &Value) -> ProbeWorld {
    let mut probes = HashMap::new();
    for (id, f) in value["probes"].as_object().unwrap() {
        probes.insert(
            id.clone(),
            ProbeFact {
                kind: f["kind"].as_str().unwrap().to_string(),
                ok_at: f["okAt"].as_u64(),
                until: f["until"].as_u64(),
                latency_ms: None,
                fail_count: f["failCount"].as_u64().unwrap_or(0),
                next_probe_at: f["nextProbeAt"].as_u64().unwrap_or(0),
            },
        );
    }
    ProbeWorld {
        probes,
        invalid: value["invalid"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<HashSet<_>>(),
        paused_owners: value["pausedOwners"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<HashSet<_>>(),
        cooling_until: value["coolingUntil"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_u64().unwrap()))
            .collect(),
    }
}

fn admission_case(prompt: u64, reserve: u64, lane: LaneState, declared: u64, learned: Option<u64>, safe: Option<u64>) -> Value {
    let outcome = admit(prompt, reserve, &lane, declared, learned, safe);
    let expect = match outcome {
        Admission::Admit => "admit",
        Admission::Invalid => "invalid",
        Admission::Cooling => "cooling",
        Admission::OwnerPaused => "owner-paused",
        Admission::NoFit => "no-fit",
    };
    json!({
        "promptTokens": prompt,
        "reserveTokens": reserve,
        "lane": { "invalid": lane.invalid, "cooling": lane.cooling, "ownerPaused": lane.owner_paused },
        "declared": declared,
        "learned": learned,
        "safeBytes": safe,
        "expect": expect,
    })
}

fn vectors() -> Value {
    let lanes = vec![
        lane("dahl/MiniMaxAI/MiniMax-M2.7", "dahl"),
        lane("freeinference/deepseek-v4-flash", "freeinference"),
        lane("agentrouter/gpt-5.6-sol", "agentrouter"),
    ];

    let mut admission = Vec::new();
    admission.push(admission_case(100_000, 16_384, LaneState::default(), 180_000, None, None));
    admission.push(admission_case(170_000, 16_384, LaneState::default(), 180_000, None, None));
    admission.push(admission_case(100_000, 16_384, LaneState { invalid: true, ..Default::default() }, 180_000, None, None));
    admission.push(admission_case(100_000, 16_384, LaneState { cooling: true, ..Default::default() }, 180_000, None, None));
    admission.push(admission_case(100_000, 16_384, LaneState { owner_paused: true, ..Default::default() }, 180_000, None, None));
    admission.push(admission_case(20_000, 16_384, LaneState::default(), 180_000, None, Some(56 * 1024)));
    admission.push(admission_case(10_000, 1_000, LaneState::default(), 180_000, None, Some(56 * 1024)));
    admission.push(admission_case(150_000, 16_384, LaneState::default(), 400_000, Some(180_000), None));
    admission.push(admission_case(150_000, 16_384, LaneState::default(), 400_000, Some(262_144), None));

    let mut backoff = Vec::new();
    for fail_count in 1..=9u64 {
        for jitter in [0.0, 0.5, 1.0] {
            backoff.push(json!({
                "failCount": fail_count,
                "jitter": jitter,
                "expectMs": probe_backoff_ms(fail_count, jitter),
            }));
        }
    }

    let selection_worlds = vec![
        // unverified wins
        world_json(
            &[
                ("dahl/MiniMaxAI/MiniMax-M2.7", fact("cooling", None, Some(NOW - 5_000), NOW, 1)),
                ("freeinference/deepseek-v4-flash", fact("ok", Some(NOW - 11 * 60_000), None, NOW, 0)),
            ],
            &[], &[], &[],
        ),
        // cooldown just expired beats stale
        world_json(
            &[
                ("dahl/MiniMaxAI/MiniMax-M2.7", fact("cooling", None, Some(NOW - 5_000), NOW, 1)),
                ("freeinference/deepseek-v4-flash", fact("ok", Some(NOW - 1_000), None, NOW, 0)),
                ("agentrouter/gpt-5.6-sol", fact("ok", Some(NOW - 1_000), None, NOW, 0)),
            ],
            &[], &[], &[],
        ),
        // every lane blocked
        world_json(
            &[],
            &["agentrouter/gpt-5.6-sol"],
            &["freeinference"],
            &[("dahl/MiniMaxAI/MiniMax-M2.7", NOW + 60_000)],
        ),
    ];
    let mut selection = Vec::new();
    for value in &selection_worlds {
        let world = build_world(value);
        let choice = select_probe_candidate(&lanes, &world, NOW, DEFAULT_VERIFIED_TTL_MS);
        selection.push(json!({
            "lanes": lanes.iter().map(|l| json!({"id": l.id, "owner": l.owner})).collect::<Vec<_>>(),
            "world": value,
            "now": NOW,
            "ttl": DEFAULT_VERIFIED_TTL_MS,
            "expect": choice.map(|c| json!({"lane": c.lane.id, "why": c.why})),
        }));
    }

    let exhaustion_worlds = vec![
        world_json(&[], &[], &[], &[("dahl/MiniMaxAI/MiniMax-M2.7", NOW + 60_000)]),
        world_json(
            &[],
            &["dahl/MiniMaxAI/MiniMax-M2.7"],
            &["freeinference"],
            &[("agentrouter/gpt-5.6-sol", NOW + 31 * 60_000)],
        ),
    ];
    let mut exhaustion = Vec::new();
    for value in &exhaustion_worlds {
        let world = build_world(value);
        exhaustion.push(json!({
            "lanes": lanes.iter().map(|l| json!({"id": l.id, "owner": l.owner})).collect::<Vec<_>>(),
            "world": value,
            "now": NOW,
            "horizon": DEFAULT_EXHAUSTED_HORIZON_MS,
            "expect": is_exhausted(&lanes, &world, NOW, DEFAULT_EXHAUSTED_HORIZON_MS),
        }));
    }

    // Rate limiter: three allowed in one minute, fourth blocked, next day resets.
    let mut steps = Vec::new();
    let mut bucket = RateBucket::default();
    for offset in [0u64, 1, 2, 3, 10] {
        let (allow, next) = rate_allow(&bucket, NOW + offset, 3, 100);
        steps.push(json!({"offsetMs": offset, "expectAllow": allow}));
        bucket = next;
    }
    let (allow_next_day, _) = rate_allow(&bucket, NOW + 86_400_000, 3, 100);
    steps.push(json!({"offsetMs": 86_400_000, "expectAllow": allow_next_day}));

    json!({
        "now": NOW,
        "admission": admission,
        "backoff": backoff,
        "selection": selection,
        "exhaustion": exhaustion,
        "rate": {"perMin": 3, "daily": 100, "steps": steps},
    })
}

#[test]
fn golden_vectors_are_current() {
    let generated = serde_json::to_string_pretty(&vectors()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/vectors.json");
    if std::env::var("VITRIOL_WRITE_VECTORS").as_deref() == Ok("1") {
        std::fs::write(&path, format!("{generated}\n")).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).expect("testdata/vectors.json missing");
    let a: Value = serde_json::from_str(&committed).unwrap();
    let b: Value = serde_json::from_str(&generated).unwrap();
    assert_eq!(a, b, "golden vectors drifted — run VITRIOL_WRITE_VECTORS=1 cargo test --test emit_vectors");
}
