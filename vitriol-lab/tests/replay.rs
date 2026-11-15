use vitriol_lab::{
    model::*,
    replay::{self, Experiment},
};

fn fixture() -> Experiment {
    serde_json::from_str(include_str!("../examples/sessions.json")).unwrap()
}

fn config() -> Config {
    fixture().candidates[1].clone()
}

fn request(id: &str, at: u64, prefix: &[u64]) -> Request {
    let mut r = fixture().trace.requests[0].clone();
    r.id = id.into();
    r.at_us = at;
    r.prefix_ids = prefix.into();
    r.state_prefix_ids = prefix.into();
    r
}

fn trace(requests: Vec<Request>) -> Trace {
    let mut t = fixture().trace;
    t.requests = requests;
    t
}

#[test]
fn golden_cold_replay_and_canonical_provenance_are_deterministic() {
    let e = fixture();
    let a = replay::replay(&e.trace, &e.candidates[0]).unwrap();
    let b = replay::replay(&e.trace, &e.candidates[0]).unwrap();
    assert_eq!(a.totals.processed_tokens, 24);
    assert_eq!(a.totals.prefill_us, 240);
    assert_eq!(a.totals.makespan_us, 350);
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&b).unwrap()
    );
    assert_eq!(a.trace_sha256, vitriol_lab::fingerprint(&e.trace).unwrap());
    assert!(!a.hardware_verified);
}

#[test]
fn a_busy_slot_cannot_publish_to_an_earlier_arrival() {
    let t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 10, &[1, 2, 3, 4]),
        request("c", 50, &[1, 2, 3, 4]),
    ]);
    let mut cfg = config();
    cfg.slots = 2;
    let result = replay::replay(&t, &cfg).unwrap();
    assert_eq!(result.requests[1].cached_tokens, 0);
    assert_eq!(
        result.requests[2].cached_tokens, 4,
        "completion precedes equal-time arrival"
    );
}

#[test]
fn queue_time_and_p95_include_waiting_for_busy_slots() {
    let t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 1, &[9, 8, 7, 6]),
    ]);
    let result = replay::replay(&t, &config()).unwrap();
    assert_eq!(result.requests[1].start_us, 50);
    assert_eq!(result.requests[1].queue_us, 49);
    assert_eq!(result.totals.p95_latency_us, 99);
}

#[test]
fn free_slot_affinity_preserves_fifo_request_order() {
    let t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 1, &[9, 8, 7, 6]),
        request("c", 100, &[9, 8, 7, 6]),
    ]);
    let mut cfg = config();
    cfg.slots = 2;
    cfg.ram_bytes = 0;
    cfg.disk_bytes = 0;
    let fifo = replay::replay(&t, &cfg).unwrap();
    cfg.dispatch = Dispatch::Affinity;
    let affinity = replay::replay(&t, &cfg).unwrap();
    assert_eq!(fifo.requests[2].cached_tokens, 0);
    assert_eq!(affinity.requests[2].cached_tokens, 4);
    assert_eq!(affinity.requests[2].slot, 1);
    assert!(affinity.totals.prefill_us < fifo.totals.prefill_us);
}

#[test]
fn overshoot_and_identity_mismatch_cannot_be_reused() {
    let mut t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 100, &[1, 2, 9]),
    ]);
    let mut cfg = config();
    assert_eq!(
        replay::replay(&t, &cfg).unwrap().requests[1].cached_tokens,
        0
    );
    cfg.rewind_tokens = 2;
    assert_eq!(
        replay::replay(&t, &cfg).unwrap().requests[1].cached_tokens,
        2
    );
    t.requests[1].layout = "other-layout".into();
    assert_eq!(
        replay::replay(&t, &cfg).unwrap().requests[1].cached_tokens,
        0
    );
    t.requests[1].layout = t.requests[0].layout.clone();
    t.requests[1].model = "other-model".into();
    assert_eq!(
        replay::replay(&t, &cfg).unwrap().requests[1].cached_tokens,
        0
    );
}

#[test]
fn protected_entries_force_skipped_admission_instead_of_eviction() {
    let mut t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 100, &[9, 8]),
        request("c", 200, &[1, 2, 3, 4]),
    ]);
    t.requests[0].protected = true;
    let mut cfg = config();
    cfg.ram_bytes = 40;
    let result = replay::replay(&t, &cfg).unwrap();
    assert_eq!(result.requests[2].cached_tokens, 4);
    assert_eq!(result.cache.evicted, 0);
    assert!(result.cache.skipped >= 1);
    assert_eq!(result.cache.peak_ram_bytes, 40);
}

#[test]
fn active_ram_readers_protect_their_backing_snapshot() {
    let mut t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 60, &[9, 8]),
        request("c", 100, &[1, 2, 3, 4, 5]),
        request("d", 300, &[6]),
        request("e", 400, &[1, 2, 3, 4, 6]),
    ]);
    t.requests[1].decode_us = 200;
    t.requests[2].decode_us = 1000;
    let mut cfg = config();
    cfg.slots = 2;
    cfg.ram_bytes = 40;
    let result = replay::replay(&t, &cfg).unwrap();
    assert_eq!(result.requests[2].cached_tokens, 4);
    assert_eq!(result.requests[4].cached_tokens, 4);
    assert!(result.cache.skipped > 0);
    assert_eq!(result.cache.peak_ram_bytes, 40);
}

#[test]
fn value_policy_keeps_expensive_recomputation_per_byte() {
    let mut t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 5000, &[9]),
        request("c", 6000, &[8]),
        request("d", 7000, &[1, 2, 3, 4]),
    ]);
    t.requests[0].prefill_us_per_token = 1000;
    t.requests[1].prefill_us_per_token = 1;
    t.requests[2].prefill_us_per_token = 1;
    let mut cfg = config();
    let lru = replay::replay(&t, &cfg).unwrap();
    cfg.eviction = Eviction::Value;
    let value = replay::replay(&t, &cfg).unwrap();
    assert_eq!(lru.requests[3].cached_tokens, 0);
    assert_eq!(value.requests[3].cached_tokens, 4);
}

#[test]
fn disk_restore_is_selected_only_when_cheaper_than_recompute() {
    let mut t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 100, &[9]),
        request("c", 200, &[1, 2, 3, 4]),
    ]);
    let mut cfg = config();
    cfg.ram_bytes = 40;
    cfg.disk_bytes = 80;
    let result = replay::replay(&t, &cfg).unwrap();
    assert_eq!(
        serde_json::to_value(&result).unwrap()["requests"][2]["source"],
        "disk"
    );
    assert_eq!(result.cache.demoted, 2);
    t.requests[2].disk_restore_us = 1000;
    assert_eq!(
        replay::replay(&t, &cfg).unwrap().requests[2].cached_tokens,
        0
    );
}

#[test]
fn heterogeneous_capacity_and_external_reservations_exclude_candidates() {
    let mut e = fixture();
    let result = replay::compare(&e).unwrap();
    assert_ne!(
        result.fastest_feasible_estimate.as_deref(),
        Some("two-slots")
    );
    let two = result.reports.iter().find(|r| r.config.slots == 2).unwrap();
    assert_eq!(two.capacity.status, "exceeds_estimate");
    assert!(two.capacity.violations.iter().all(|v| v.gpu == "small"));
    e.trace.requests[0].gpu_external_bytes = Some(vec![50, 0]);
    assert_eq!(
        replay::replay(&e.trace, &config()).unwrap().capacity.status,
        "exceeds_estimate"
    );
    e.trace.gpus.clear();
    e.trace.requests[0].gpu_external_bytes = None;
    assert!(replay::compare(&e)
        .unwrap()
        .fastest_feasible_estimate
        .is_none());
}

#[test]
fn invalid_traces_and_numeric_overflow_fail_explicitly() {
    let mut e = fixture();
    e.trace.requests[1].id = e.trace.requests[0].id.clone();
    assert!(replay::compare(&e).is_err());
    let mut e = fixture();
    e.trace.requests[1].at_us = 0;
    e.trace.requests[2].at_us = 0;
    e.trace.requests[3].at_us = 0;
    e.trace.requests[4].at_us = 0;
    e.trace.requests[0].prefill_us_per_token = u64::MAX;
    assert!(replay::compare(&e).err().unwrap().contains("overflow"));
}

#[test]
fn explicit_reprocessing_and_no_snapshot_still_preserve_slot_state() {
    let mut t = trace(vec![
        request("a", 0, &[1, 2, 3, 4]),
        request("b", 100, &[1, 2, 3, 4]),
    ]);
    t.requests[0].snapshot_bytes = 0;
    t.requests[1].reprocess_tokens = 1;
    let result = replay::replay(&t, &config()).unwrap();
    assert_eq!(result.requests[1].processed_tokens, 1);
    assert_eq!(result.requests[1].cached_tokens, 3);
}
