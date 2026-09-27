// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md, M2). The arbiter: placement
// decision for one chat request. Heuristic v0 - no model calls yet (M5).
// Laws (order is binding):
//   1. privacy: sensitive/confidential NEVER leave the local lane
//      (fail-closed: a failed scan also stays local)
//   2. stickiness: a pinned session stays on its lane while healthy,
//      and returns to local only under the low watermark (no flapping)
//   3. pressure: overflow only above the high watermark
//   4. cost: overflow prefers free lanes; paid lanes are M3
// Everything else equal: local.

use crate::affinity::SessionTable;
use crate::config::{LaneKind, Registry};
use crate::pressure::PressureBoard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// local lane had headroom below the high watermark
    LocalHeadroom,
    /// session affinity kept the lane
    Affinity,
    /// pressure fell below the low watermark: session recalled to local
    Recall,
    /// pressure overflowed: session spilled to a non-local lane
    Overflow,
    /// privacy class or scan failure forbids leaving local
    PrivacyGate,
    /// overflow wanted but no eligible non-local lane: queue on local
    LocalQueue,
}

impl Cause {
    pub fn as_str(&self) -> &'static str {
        match self {
            Cause::LocalHeadroom => "local-headroom",
            Cause::Affinity => "affinity",
            Cause::Recall => "recall",
            Cause::Overflow => "overflow",
            Cause::PrivacyGate => "privacy-gate",
            Cause::LocalQueue => "local-queue",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// busy_ratio above which overflow may trigger (0..=1]
    pub high: f64,
    /// busy_ratio below which pinned sessions are recalled to local
    pub low: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            high: 0.99,
            low: 0.5,
        }
    }
}

impl Thresholds {
    pub fn from_env() -> Self {
        let f = |key: &str, dflt: f64| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(dflt)
        };
        Self {
            high: f("DEMIURGE_HIGH_WATERMARK", 0.99),
            low: f("DEMIURGE_LOW_WATERMARK", 0.5),
        }
    }
}

pub struct Arbiter {
    pub registry: Registry,
    pub board: std::sync::Arc<PressureBoard>,
    pub sessions: SessionTable,
    pub thresholds: Thresholds,
}

pub struct Decision {
    pub lane_id: String,
    pub cause: Cause,
}

impl Arbiter {
    /// Decide the lane for one request. `privacy` is None when the request
    /// never got scanned (non-chat path) - no scan, no overflow.
    /// `local_unhealthy` lets overflow fire even below the watermark when
    /// the local lane is dead.
    pub fn decide(
        &self,
        session: Option<&str>,
        privacy: Option<privacy::PrivacyClass>,
        local_unhealthy: bool,
    ) -> Decision {
        let local = self.registry.local();
        let pressure = self.board.get(&local.id);

        // Law 1: privacy gate. Fail-closed: scan error also stays local.
        let gated = matches!(
            privacy,
            Some(privacy::PrivacyClass::Sensitive) | Some(privacy::PrivacyClass::Confidential)
        ) || privacy.is_none();

        let local_ok = !local_unhealthy
            && pressure
                .as_ref()
                .map(|p| p.healthy)
                // no poll data yet (cold start): assume local fine, don't overflow
                .unwrap_or(true);
        let busy = pressure.as_ref().map(|p| p.busy_ratio()).unwrap_or(0.0);
        let overflow_possible = !gated && self.registry.overflow_lane().is_some();

        // Law 1 (strictly first): a gated request never rides an affinity
        // pin to a non-local lane - caught live in the M2 smoke (audit row
        // 2: sensitive request took cause=affinity to zen).
        if gated {
            self.sessions.clear_if(session, &local.id);
            return Decision {
                lane_id: local.id.clone(),
                cause: Cause::PrivacyGate,
            };
        }

        // Law 2: stickiness - an existing non-local pin wins while healthy.
        if let Some(sid) = session {
            if let Some(binding) = self.sessions.get(sid) {
                if binding.lane != local.id {
                    let lane_alive = self
                        .registry
                        .get(&binding.lane)
                        .map(|l| {
                            // cloud lanes report no pressure - absence = alive
                            self.board.get(&l.id).map(|p| p.healthy).unwrap_or(true)
                        })
                        .unwrap_or(false);
                    if lane_alive {
                        if busy < self.thresholds.low {
                            // Law 2 recall: pressure gone, come home
                            self.sessions.clear(sid);
                            return Decision {
                                lane_id: local.id.clone(),
                                cause: Cause::Recall,
                            };
                        }
                        return Decision {
                            lane_id: binding.lane.clone(),
                            cause: Cause::Affinity,
                        };
                    }
                    self.sessions.clear(sid);
                }
            }
        }

        // no-overflow setups: stay local regardless.
        if !overflow_possible {
            self.pin_session(session, &local.id);
            return Decision {
                lane_id: local.id.clone(),
                cause: Cause::LocalQueue,
            };
        }

        // Law 3: overflow above the high watermark (or dead local lane).
        let overflow_now = local_unhealthy || !local_ok || busy >= self.thresholds.high;
        if overflow_now {
            if let Some(lane) = self.registry.overflow_lane() {
                self.pin_session(session, &lane.id);
                return Decision {
                    lane_id: lane.id.clone(),
                    cause: Cause::Overflow,
                };
            }
        }

        self.pin_session(session, &local.id);
        Decision {
            lane_id: local.id.clone(),
            cause: if local_ok && busy < self.thresholds.high {
                Cause::LocalHeadroom
            } else {
                Cause::LocalQueue
            },
        }
    }

    fn pin_session(&self, session: Option<&str>, lane: &str) {
        if let Some(sid) = session {
            self.sessions.pin(sid, lane);
        }
    }
}

impl Registry {
    /// The first healthy-eligible overflow lane (cloud-free today; node +
    /// cloud-paid join in M3/M4).
    pub fn overflow_lane(&self) -> Option<&crate::config::Lane> {
        self.lanes
            .iter()
            .find(|l| l.overflow && l.kind == LaneKind::CloudFree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LaneKind;
    use std::sync::Arc;

    fn board_with(busy: u32, total: u32, healthy: bool) -> Arc<PressureBoard> {
        let b = PressureBoard::new();
        b.set(
            "lapis",
            crate::pressure::Pressure {
                busy_slots: busy,
                total_slots: total,
                deferred: 0,
                tokens_per_second: None,
                healthy,
                updated_at_ms: 0,
            },
        );
        b.set(
            "zen",
            crate::pressure::Pressure {
                busy_slots: 0,
                total_slots: 1,
                deferred: 0,
                tokens_per_second: None,
                healthy: true,
                updated_at_ms: 0,
            },
        );
        b
    }

    fn arbiter(board: Arc<PressureBoard>) -> Arbiter {
        let json = r#"{
            "lanes": [
                { "id": "lapis", "kind": "local", "endpoint": "http://127.0.0.1:8279", "always_preferred": true },
                { "id": "zen", "kind": "cloud-free", "endpoint": "https://zen.example/v1", "models": ["m1"], "overflow": true }
            ]
        }"#;
        Arbiter {
            registry: Registry::parse(json).unwrap(),
            board,
            sessions: SessionTable::new(1800),
            thresholds: Thresholds::default(),
        }
    }

    #[test]
    fn quiet_engine_stays_local() {
        let a = arbiter(board_with(0, 2, true));
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(d.lane_id, "lapis");
        assert_eq!(d.cause, Cause::LocalHeadroom);
    }

    #[test]
    fn saturated_engine_overflows() {
        let a = arbiter(board_with(2, 2, true));
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(d.lane_id, "zen");
        assert_eq!(d.cause, Cause::Overflow);
    }

    #[test]
    fn sensitive_never_leaves_even_saturated() {
        for class in [
            privacy::PrivacyClass::Sensitive,
            privacy::PrivacyClass::Confidential,
        ] {
            let a = arbiter(board_with(2, 2, true));
            let d = a.decide(Some("s1"), Some(class), false);
            assert_eq!(d.lane_id, "lapis");
            assert_eq!(d.cause, Cause::PrivacyGate);
        }
    }

    #[test]
    fn failed_scan_stays_local_fail_closed() {
        let a = arbiter(board_with(2, 2, true));
        let d = a.decide(Some("s1"), None, false);
        assert_eq!(d.lane_id, "lapis");
        assert_eq!(d.cause, Cause::PrivacyGate);
    }

    #[test]
    fn affinity_sticks_while_pressure_holds() {
        let board = board_with(2, 2, true);
        let a = arbiter(board);
        a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        // second saturated decision: session is now pinned to zen
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(d.lane_id, "zen");
        assert_eq!(d.cause, Cause::Affinity);
    }

    #[test]
    fn recall_below_low_watermark() {
        let board = board_with(2, 2, true);
        let a = arbiter(board);
        a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(a.sessions.get("s1").unwrap().lane, "zen");
        // pressure drops
        a.board.set(
            "lapis",
            crate::pressure::Pressure {
                busy_slots: 0,
                total_slots: 2,
                deferred: 0,
                tokens_per_second: None,
                healthy: true,
                updated_at_ms: 0,
            },
        );
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(d.lane_id, "lapis");
        assert_eq!(d.cause, Cause::Recall);
        assert!(a.sessions.get("s1").is_none(), "pin cleared on recall");
    }

    #[test]
    fn dead_local_overflows_even_below_watermark() {
        let a = arbiter(board_with(0, 2, false));
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), true);
        assert_eq!(d.lane_id, "zen");
        assert_eq!(d.cause, Cause::Overflow);
    }

    #[test]
    fn no_overflow_lane_queues_locally() {
        let json = r#"{
            "lanes": [
                { "id": "lapis", "kind": "local", "endpoint": "http://127.0.0.1:8279", "always_preferred": true }
            ]
        }"#;
        let a = Arbiter {
            registry: Registry::parse(json).unwrap(),
            board: board_with(2, 2, true),
            sessions: SessionTable::new(1800),
            thresholds: Thresholds::default(),
        };
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(d.lane_id, "lapis");
        assert_eq!(d.cause, Cause::LocalQueue);
    }

    #[test]
    fn node_lane_is_not_overflow_until_m4() {
        let json = r#"{
            "lanes": [
                { "id": "lapis", "kind": "local", "endpoint": "http://127.0.0.1:8279", "always_preferred": true },
                { "id": "boxb", "kind": "node", "endpoint": "http://b:8280", "kv": true },
                { "id": "zen", "kind": "cloud-free", "endpoint": "https://z/v1", "models": ["m1"], "overflow": true }
            ]
        }"#;
        let a = Arbiter {
            registry: Registry::parse(json).unwrap(),
            board: board_with(2, 2, true),
            sessions: SessionTable::new(1800),
            thresholds: Thresholds::default(),
        };
        let d = a.decide(Some("s1"), Some(privacy::PrivacyClass::Safe), false);
        assert_eq!(d.lane_id, "zen");
        assert_eq!(
            a.registry.overflow_lane().unwrap().kind,
            LaneKind::CloudFree
        );
    }
}
