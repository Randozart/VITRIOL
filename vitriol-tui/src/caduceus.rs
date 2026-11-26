//! Loopback router telemetry and the operator's per-lane unpark action.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use ureq::Agent;

#[derive(Debug, Clone, Default)]
pub struct Status {
    router: Router,
    pub lanes: Vec<Lane>,
    pub catalog_count: Option<usize>,
}

#[derive(Debug, Clone, Default)]
enum Router {
    #[default]
    Checking,
    Unreachable,
    Online {
        requested: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Lane {
    pub model: String,
    pub in_route: bool,
    pub remaining: Option<u64>,
    pub reason: Option<String>,
}

impl Status {
    pub fn summary(&self) -> String {
        let requested = match &self.router {
            Router::Checking => return "Checking router…".into(),
            Router::Unreachable => return "Router unreachable".into(),
            Router::Online { requested } => requested,
        };
        let lanes = &self.lanes;
        let route = lanes.iter().filter(|lane| lane.in_route);
        let count = route.clone().count();
        let eligible = route
            .clone()
            .filter(|lane| lane.remaining.is_none())
            .count();
        let scope = requested.as_deref().unwrap_or("Configured fallbacks");
        if count == 0 {
            return "Router online · no routes reported".into();
        }
        let mut summary = format!("{scope} · {eligible}/{count} eligible");
        if let Some(seconds) = route.filter_map(|lane| lane.remaining).min() {
            summary.push_str(&format!(" · next cooldown ends in {seconds}s"));
        }
        summary
    }
}

pub(crate) fn status(body: &Value) -> Option<Status> {
    if body["ok"].as_bool() != Some(true) {
        return None;
    }
    let fallbacks = body["fallbacks"].as_array()?;
    let parked = body["parked"].as_array()?;
    let requested = body["lastRequested"].as_str().map(str::to_owned);
    let mut lanes: Vec<Lane> = Vec::new();
    for model in requested
        .iter()
        .map(String::as_str)
        .chain(fallbacks.iter().filter_map(Value::as_str))
    {
        if !lanes.iter().any(|lane| lane.model == model) {
            lanes.push(Lane {
                model: model.into(),
                in_route: true,
                remaining: None,
                reason: None,
            });
        }
    }
    for entry in parked {
        let (Some(model), Some(seconds)) = (entry["model"].as_str(), entry["seconds"].as_u64())
        else {
            continue;
        };
        let reason = entry["reason"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(lane) = lanes.iter_mut().find(|lane| lane.model == model) {
            lane.remaining = Some(seconds.max(1));
            lane.reason = reason;
        } else {
            lanes.push(Lane {
                model: model.into(),
                in_route: false,
                remaining: Some(seconds.max(1)),
                reason,
            });
        }
    }
    Some(Status {
        router: Router::Online { requested },
        lanes,
        catalog_count: None,
    })
}

#[derive(Default)]
pub struct CatalogCache {
    stamp: Option<(SystemTime, u64)>,
    ids: Option<BTreeSet<String>>,
}

impl CatalogCache {
    pub fn merge(&mut self, home: &Path, mut snapshot: Status) -> Status {
        self.read(&home.join(".vitriol/officina/caduceus-models.json"));
        if let Some(ids) = &self.ids {
            snapshot.catalog_count = Some(ids.len());
            let live_ids: HashSet<_> = snapshot
                .lanes
                .iter()
                .map(|lane| lane.model.as_str())
                .collect();
            let additional: Vec<_> = ids
                .iter()
                .filter(|id| !live_ids.contains(id.as_str()))
                .map(|id| Lane {
                    model: id.clone(),
                    in_route: false,
                    remaining: None,
                    reason: None,
                })
                .collect();
            snapshot.lanes.extend(additional);
        }
        snapshot
    }

    fn read(&mut self, path: &Path) {
        let Ok(meta) = fs::metadata(path) else { return };
        let stamp = meta.modified().ok().map(|time| (time, meta.len()));
        if stamp.is_some() && stamp == self.stamp {
            return;
        }
        if let Some(ids) = fs::read(path).ok().and_then(|bytes| catalog_ids(&bytes)) {
            self.stamp = stamp;
            self.ids = Some(ids);
        }
    }
}

fn catalog_ids(bytes: &[u8]) -> Option<BTreeSet<String>> {
    let body: Value = serde_json::from_slice(bytes).ok()?;
    Some(
        body["data"]
            .as_array()?
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn base() -> String {
    let port = std::env::var("CADUCEUS_ROUTER_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(20129);
    format!("http://127.0.0.1:{port}")
}

pub fn agent() -> Agent {
    Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(500)))
        .proxy(None)
        .max_redirects(0)
        .build()
        .into()
}

pub fn poll(agent: &Agent) -> Status {
    agent
        .get(format!("{}/health", base()))
        .call()
        .ok()
        .and_then(|mut response| response.body_mut().read_json::<Value>().ok())
        .and_then(|body| status(&body))
        .unwrap_or_else(|| Status {
            router: Router::Unreachable,
            ..Status::default()
        })
}

pub fn unpark(model: &str) -> Result<(), String> {
    unpark_at(&base(), model)
}

fn unpark_at(base: &str, model: &str) -> Result<(), String> {
    let result = agent()
        .delete(format!("{base}/park"))
        .query("model", model)
        .call();
    let mut reply = match result {
        Ok(reply) => reply,
        Err(ureq::Error::StatusCode(404)) => return Err(format!("{model} is no longer parked")),
        Err(error) => return Err(format!("Could not unpark {model} · {error}")),
    };
    let body = reply
        .body_mut()
        .read_json::<Value>()
        .map_err(|error| error.to_string())?;
    if body["ok"].as_bool() == Some(true) && body["unparked"].as_str() == Some(model) {
        Ok(())
    } else {
        Err(format!("Router did not confirm unpark for {model}"))
    }
}

pub fn log_path(home: &Path) -> PathBuf {
    let state = std::env::var_os("CADUCEUS_ROUTER_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".vitriol/officina/caduceus-router-state.json"));
    state.with_file_name("caduceus-router-reroutes.jsonl")
}

pub fn compact_event(line: &str) -> Option<String> {
    let event: Value = serde_json::from_str(line).ok()?;
    let time = event["ts"].as_str().unwrap_or("");
    if let Some(model) = event["unparked"].as_str() {
        return Some(format!("{time} · {model} · unparked"));
    }
    let model = event["model"].as_str().unwrap_or("router");
    let details = if event["served"].as_bool() == Some(true) {
        "served after reroute".to_owned()
    } else if let Some(reason) = event["reason"].as_str() {
        reason.to_owned()
    } else {
        format!("HTTP {}", event["status"].as_u64()?)
    };
    Some(format!("{time} · {model} · {details}"))
}

pub fn is_route_event(line: &str, model: &str) -> bool {
    serde_json::from_str::<Value>(line).is_ok_and(|event| {
        event["model"].as_str() == Some(model) || event["unparked"].as_str() == Some(model)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_merges_all_ids_and_keeps_live_status_separate() {
        let bytes = br#"{"data":[{"id":"zai/glm-flash"},{"id":"a/glm"},{"id":"zai/glm-flash"},{"id":"auto"}]}"#;
        let mut catalog = CatalogCache {
            ids: catalog_ids(bytes),
            ..Default::default()
        };
        let live = status(&json!({
            "ok": true, "lastRequested": "a/glm", "fallbacks": [],
            "parked": [{"model":"a/glm", "seconds":30}]
        }))
        .unwrap();
        let merged = catalog.merge(Path::new("/dev/null"), live);
        assert_eq!(merged.catalog_count, Some(3));
        assert_eq!(
            merged.summary(),
            "a/glm · 0/1 eligible · next cooldown ends in 30s"
        );
        assert_eq!(
            merged
                .lanes
                .iter()
                .map(|lane| lane.model.as_str())
                .collect::<Vec<_>>(),
            ["a/glm", "auto", "zai/glm-flash"]
        );
        assert!(merged.lanes[1..]
            .iter()
            .all(|lane| !lane.in_route && lane.remaining.is_none()));
        let down = catalog.merge(
            Path::new("/dev/null"),
            Status {
                router: Router::Unreachable,
                ..Status::default()
            },
        );
        assert_eq!(down.summary(), "Router unreachable");
        assert_eq!(down.lanes.len(), 3);
    }

    #[test]
    fn summary_scopes_cooldown_to_the_recent_request_and_fallbacks() {
        let status = status(&json!({
            "ok": true, "lastRequested": "a/glm", "fallbacks": ["a/glm", "b/glm"],
            "parked": [
                {"model": "a/glm", "seconds": 30, "reason": "HTTP 429"},
                {"model": "b/glm", "seconds": 40, "reason": "HTTP 503"},
                {"model": "old/route", "seconds": 2}
            ]
        }))
        .unwrap();
        assert_eq!(
            status.summary(),
            "a/glm · 0/2 eligible · next cooldown ends in 30s"
        );
        assert_eq!(status.lanes[0].reason.as_deref(), Some("HTTP 429"));
        assert!(!status.lanes[2].in_route);
    }

    #[test]
    fn older_router_uses_fallbacks_and_ignores_unrelated_health_responses() {
        let current = status(&json!({"ok": true, "fallbacks": ["b/glm"], "parked": []})).unwrap();
        assert_eq!(current.summary(), "Configured fallbacks · 1/1 eligible");
        assert!(status(&json!({"ok": true})).is_none());
        assert_eq!(
            Status {
                router: Router::Unreachable,
                ..Status::default()
            }
            .summary(),
            "Router unreachable"
        );
    }
}
