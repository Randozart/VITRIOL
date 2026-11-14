//! vitriol-core-rpc — line-delimited JSON-RPC over stdio (Phase 2 sidecar).
//!
//! One JSON object per line in, one per line out:
//!   {"id":1,"method":"admit","params":{...}} -> {"id":1,"result":{...}}
//!   {"id":2,"method":"probe_plan","params":{...}}
//!   {"id":3,"method":"parse_error","params":{"text":"..."}}
//!
//! The TS client (`officina/.pi/extensions/_shared/core-rpc.ts`) fails open:
//! any spawn/parse/timeout problem falls back to the TS implementation, so a
//! broken sidecar can never block a turn.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

use serde::Deserialize;
use serde_json::{json, Value};

use vitriol_core::scheduler::{
    is_exhausted, probe_backoff_ms, rate_allow, select_probe_candidate, ProbeFact, ProbeLane,
    ProbeWorld, RateBucket, DEFAULT_EXHAUSTED_HORIZON_MS, DEFAULT_VERIFIED_TTL_MS,
};
use vitriol_core::{admit, parse_provider_error, Admission, LaneState};

#[derive(Debug, Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<u64>,
    method: String,
    #[serde(default)]
    params: Value,
}

fn admission_name(outcome: Admission) -> &'static str {
    match outcome {
        Admission::Admit => "admit",
        Admission::Invalid => "invalid",
        Admission::Cooling => "cooling",
        Admission::OwnerPaused => "owner-paused",
        Admission::NoFit => "no-fit",
    }
}

fn method_admit(params: &Value) -> Result<Value, String> {
    let prompt = params
        .get("promptTokens")
        .and_then(Value::as_u64)
        .ok_or("promptTokens required")?;
    let reserve = params
        .get("reserveTokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let lanes = params
        .get("lanes")
        .and_then(Value::as_array)
        .ok_or("lanes required")?;
    let mut decisions = serde_json::Map::new();
    for lane in lanes {
        let id = lane.get("id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let state = LaneState {
            invalid: lane
                .get("invalid")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            cooling: lane
                .get("cooling")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            owner_paused: lane
                .get("ownerPaused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        let decision = admit(
            prompt,
            reserve,
            &state,
            lane.get("declared").and_then(Value::as_u64).unwrap_or(0),
            lane.get("learned").and_then(Value::as_u64),
            lane.get("safeBytes").and_then(Value::as_u64),
        );
        decisions.insert(id.to_string(), json!(admission_name(decision)));
    }
    Ok(json!({ "decisions": decisions }))
}

fn parse_world(params: &Value) -> ProbeWorld {
    let mut probes = HashMap::new();
    if let Some(map) = params.get("probes").and_then(Value::as_object) {
        for (id, f) in map {
            probes.insert(
                id.clone(),
                ProbeFact {
                    kind: f
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    ok_at: f.get("okAt").and_then(Value::as_u64),
                    until: f.get("until").and_then(Value::as_u64),
                    latency_ms: f.get("latencyMs").and_then(Value::as_u64),
                    fail_count: f.get("failCount").and_then(Value::as_u64).unwrap_or(0),
                    next_probe_at: f.get("nextProbeAt").and_then(Value::as_u64).unwrap_or(0),
                },
            );
        }
    }
    let strings = |key: &str| -> HashSet<String> {
        params
            .get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut cooling_until = HashMap::new();
    if let Some(map) = params.get("coolingUntil").and_then(Value::as_object) {
        for (id, until) in map {
            if let Some(ms) = until.as_u64() {
                cooling_until.insert(id.clone(), ms);
            }
        }
    }
    ProbeWorld {
        probes,
        invalid: strings("invalid"),
        paused_owners: strings("pausedOwners"),
        cooling_until,
    }
}

fn parse_lanes(params: &Value) -> Vec<ProbeLane> {
    params
        .get("lanes")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|lane| {
                    let id = lane.get("id").and_then(Value::as_str)?;
                    Some(ProbeLane {
                        id: id.to_string(),
                        owner: lane
                            .get("owner")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_lowercase(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn method_probe_plan(params: &Value) -> Result<Value, String> {
    let now = params
        .get("now")
        .and_then(Value::as_u64)
        .ok_or("now required")?;
    let ttl = params
        .get("ttl")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_VERIFIED_TTL_MS);
    let horizon = params
        .get("horizon")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_EXHAUSTED_HORIZON_MS);
    let per_min = params.get("perMin").and_then(Value::as_u64).unwrap_or(6);
    let daily = params.get("daily").and_then(Value::as_u64).unwrap_or(2000);
    let lanes = parse_lanes(params);
    let world = parse_world(params);
    let bucket: RateBucket = params
        .get("bucket")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    if is_exhausted(&lanes, &world, now, horizon) {
        return Ok(json!({
            "action": "exhausted",
            "backoffMs": probe_backoff_ms(1, 1.0),
            "bucket": bucket,
        }));
    }
    let Some(choice) = select_probe_candidate(&lanes, &world, now, ttl) else {
        return Ok(json!({ "action": "wait", "bucket": bucket }));
    };
    let (allow, next_bucket) = rate_allow(&bucket, now, per_min, daily);
    if !allow {
        return Ok(json!({ "action": "wait", "bucket": next_bucket }));
    }
    Ok(json!({
        "action": "probe",
        "lane": { "id": choice.lane.id, "owner": choice.lane.owner },
        "why": choice.why,
        "bucket": next_bucket,
    }))
}

fn method_parse_error(params: &Value) -> Result<Value, String> {
    let text = params.get("text").and_then(Value::as_str).unwrap_or("");
    let facts = parse_provider_error(text);
    Ok(json!({
        "contextLimit": facts.context_limit,
        "overflowTokens": facts.overflow_tokens,
        "cooldownMs": facts.cooldown_ms,
        "pauseMs": facts.pause_ms,
        "safeBytes": facts.safe_bytes,
        "missingModel": facts.missing_model,
        "alternatives": facts.alternatives,
    }))
}

fn handle(request: Request) -> Value {
    let result = match request.method.as_str() {
        "admit" => method_admit(&request.params),
        "probe_plan" => method_probe_plan(&request.params),
        "parse_error" => method_parse_error(&request.params),
        "ping" => Ok(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })),
        other => Err(format!("unknown method: {other}")),
    };
    match result {
        Ok(value) => json!({ "id": request.id, "result": value }),
        Err(message) => json!({ "id": request.id, "error": message }),
    }
}

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(request),
            Err(error) => json!({ "id": Value::Null, "error": format!("bad request: {error}") }),
        };
        writeln!(out, "{response}")?;
        out.flush()?;
    }
    Ok(())
}
