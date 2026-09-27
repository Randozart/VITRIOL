// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). Pressure snapshot: the signal
// the arbiter will gate placement on (M2). M1 ships the poller + parsers;
// the decision layer consumes this in M2.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Snapshot of one lane's engine pressure. `healthy` flips false when the
/// last poll failed or data went stale (stale-downweight arrives in M2).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pressure {
    pub busy_slots: u32,
    pub total_slots: u32,
    pub deferred: u32,
    pub tokens_per_second: Option<f64>,
    pub healthy: bool,
    pub updated_at_ms: i64,
}

impl Pressure {
    pub fn busy_ratio(&self) -> f64 {
        if self.total_slots == 0 {
            return 0.0;
        }
        f64::from(self.busy_slots) / f64::from(self.total_slots)
    }
}

/// Shared per-lane pressure map (poller writes, router reads).
#[derive(Debug, Default)]
pub struct PressureBoard {
    inner: RwLock<HashMap<String, Pressure>>,
}

impl PressureBoard {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set(&self, lane: &str, snap: Pressure) {
        if let Ok(mut w) = self.inner.write() {
            w.insert(lane.to_string(), snap);
        }
    }

    pub fn get(&self, lane: &str) -> Option<Pressure> {
        self.inner.read().ok().and_then(|m| m.get(lane).cloned())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Count busy/total slots from a llama.cpp `/slots` JSON array.
pub fn parse_slots(body: &str) -> Option<(u32, u32)> {
    let slots: Vec<serde_json::Value> = serde_json::from_str(body).ok()?;
    let total = slots.len() as u32;
    let busy = slots
        .iter()
        .filter(|s| {
            s.get("is_processing")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .count() as u32;
    Some((busy, total))
}

/// Pull one numeric gauge out of a prometheus text body (`name value`).
pub fn metric_value(body: &str, name: &str) -> Option<f64> {
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix(name) {
            let val = rest.trim();
            if let Ok(v) = val.parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

/// Assemble a snapshot from raw `/slots` + `/metrics` bodies.
pub fn snapshot_from(slots_body: &str, metrics_body: &str) -> Option<Pressure> {
    let (busy, total) = parse_slots(slots_body)?;
    Some(Pressure {
        busy_slots: busy,
        total_slots: total,
        deferred: metric_value(metrics_body, "llamacpp:requests_deferred").unwrap_or(0.0) as u32,
        tokens_per_second: metric_value(metrics_body, "llamacpp:tokens_per_second"),
        healthy: true,
        updated_at_ms: now_ms(),
    })
}

/// Mark every known lane stale in place (called when a poll fails).
fn mark_unhealthy(board: &PressureBoard, lane: &str) {
    let mut p = board.get(lane).unwrap_or_default();
    p.healthy = false;
    p.updated_at_ms = now_ms();
    board.set(lane, p);
}

/// Spawn the poll loop: one pass over all non-cloud lanes per tick.
/// Failures never panic; they just flip the lane unhealthy.
pub fn spawn_poller(
    board: Arc<PressureBoard>,
    registry: crate::config::Registry,
    client: reqwest::Client,
    tick: std::time::Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(tick);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            for lane in registry.lanes.iter().filter(|l| {
                matches!(
                    l.kind,
                    crate::config::LaneKind::Local | crate::config::LaneKind::Node
                )
            }) {
                let base = lane.endpoint.trim_end_matches('/');
                let slots_fut = client.get(format!("{base}/slots")).send();
                let metrics_fut = client.get(format!("{base}/metrics")).send();

                let (slots_res, metrics_res) = tokio::join!(slots_fut, metrics_fut);
                let snap = async {
                    let slots = slots_res.ok()?.text().await.ok()?;
                    let metrics = metrics_res.ok()?.text().await.ok()?;
                    snapshot_from(&slots, &metrics)
                }
                .await;

                match snap {
                    Some(snap) => board.set(&lane.id, snap),
                    None => mark_unhealthy(&board, &lane.id),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_slots_busy_ratio() {
        let body = r#"[{"id":0,"is_processing":true},{"id":1,"is_processing":false}]"#;
        assert_eq!(parse_slots(body), Some((1, 2)));
    }

    #[test]
    fn parses_slots_garbage_is_none() {
        assert_eq!(parse_slots("<html>"), None);
    }

    #[test]
    fn extracts_prometheus_gauge() {
        let body = "# HELP llamacpp:requests_deferred x\n# TYPE llamacpp:requests_deferred gauge\nllamacpp:requests_deferred 3\n";
        assert_eq!(metric_value(body, "llamacpp:requests_deferred"), Some(3.0));
        assert_eq!(metric_value(body, "llamacpp:absent"), None);
    }

    #[test]
    fn snapshot_assembles() {
        let slots = r#"[{"id":0,"is_processing":true},{"id":1,"is_processing":true}]"#;
        let metrics = "llamacpp:requests_deferred 5\nllamacpp:tokens_per_second 12.5\n";
        let snap = snapshot_from(slots, metrics).expect("snapshot");
        assert_eq!(snap.busy_slots, 2);
        assert_eq!(snap.total_slots, 2);
        assert_eq!(snap.deferred, 5);
        assert_eq!(snap.tokens_per_second, Some(12.5));
        assert!(snap.healthy);
        assert!((snap.busy_ratio() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn board_roundtrip_and_unhealthy() {
        let board = PressureBoard::new();
        let slots = r#"[{"id":0,"is_processing":false}]"#;
        let snap = snapshot_from(slots, "").unwrap();
        board.set("lapis", snap);
        assert_eq!(board.get("lapis").unwrap().busy_slots, 0);
        mark_unhealthy(&board, "lapis");
        assert!(!board.get("lapis").unwrap().healthy);
        assert!(board.get("absent").is_none());
    }
}
