//! Cloud-probe scheduler (mirrors officina/.pi/extensions/_shared/probe-scheduler.ts).
//!
//! Pure policy: what to probe next, backoff after failures, exhaustion and the
//! rate limits. Golden-vector parity with TS is the Phase-2 gate (I2).

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProbeFact {
    pub kind: String,
    pub ok_at: Option<u64>,
    pub until: Option<u64>,
    pub latency_ms: Option<u64>,
    pub fail_count: u64,
    pub next_probe_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeLane {
    pub id: String,
    pub owner: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProbeWorld {
    pub probes: HashMap<String, ProbeFact>,
    pub invalid: HashSet<String>,
    pub paused_owners: HashSet<String>,
    pub cooling_until: HashMap<String, u64>,
}

pub const BACKOFF_MS: [u64; 5] = [30_000, 60_000, 120_000, 300_000, 600_000];
pub const DEFAULT_VERIFIED_TTL_MS: u64 = 10 * 60_000;
pub const DEFAULT_EXHAUSTED_HORIZON_MS: u64 = 30 * 60_000;

/// Ladder with ±20% jitter (`jitter` in 0..=1, injected for determinism).
pub fn probe_backoff_ms(fail_count: u64, jitter: f64) -> u64 {
    let idx = fail_count
        .saturating_sub(1)
        .min(BACKOFF_MS.len() as u64 - 1) as usize;
    let factor = 0.8 + jitter.clamp(0.0, 1.0) * 0.4;
    (BACKOFF_MS[idx] as f64 * factor).round() as u64
}

pub fn is_verified(
    lane_id: &str,
    probes: &HashMap<String, ProbeFact>,
    now: u64,
    ttl_ms: u64,
) -> bool {
    match probes.get(lane_id) {
        Some(fact) if fact.kind == "ok" => fact
            .ok_at
            .map(|t| now.saturating_sub(t) < ttl_ms)
            .unwrap_or(false),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeChoice {
    pub lane: ProbeLane,
    pub why: &'static str,
}

fn blocked(lane: &ProbeLane, world: &ProbeWorld, now: u64) -> bool {
    if world.invalid.contains(&lane.id) || world.paused_owners.contains(&lane.owner) {
        return true;
    }
    if world.cooling_until.get(&lane.id).copied().unwrap_or(0) > now + 1_000 {
        return true;
    }
    matches!(world.probes.get(&lane.id), Some(f) if f.next_probe_at > now)
}

/// Priority: unverified → cooldown expired → stale → retry due.
pub fn select_probe_candidate(
    lanes: &[ProbeLane],
    world: &ProbeWorld,
    now: u64,
    ttl_ms: u64,
) -> Option<ProbeChoice> {
    let mut best: Option<(u8, u32, ProbeChoice)> = None;
    for lane in lanes {
        if blocked(lane, world, now) {
            continue;
        }
        let (priority, why) = match world.probes.get(&lane.id) {
            None => (0u8, "unverified"),
            Some(fact) if fact.kind == "ok" => {
                let ok = fact.ok_at.unwrap_or(0);
                if now.saturating_sub(ok) >= ttl_ms {
                    (2u8, "stale")
                } else {
                    continue; // fresh verification — leave it alone
                }
            }
            Some(fact) => {
                let until = fact.until.unwrap_or(0);
                if until > 0 && until <= now + 30_000 {
                    (1u8, "cooldown-expired")
                } else if fact.next_probe_at <= now {
                    (3u8, "retry")
                } else {
                    continue;
                }
            }
        };
        let rank = stable_hash(&lane.id);
        let candidate = ProbeChoice {
            lane: lane.clone(),
            why,
        };
        match &best {
            Some((bp, br, _)) if (*bp, *br) <= (priority, rank) => {}
            _ => best = Some((priority, rank, candidate)),
        }
    }
    best.map(|(_, _, choice)| choice)
}

fn stable_hash(text: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for byte in text.as_bytes() {
        h ^= u32::from(*byte);
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// True when no lane can plausibly recover within the horizon.
pub fn is_exhausted(lanes: &[ProbeLane], world: &ProbeWorld, now: u64, horizon_ms: u64) -> bool {
    if lanes.is_empty() {
        return true;
    }
    for lane in lanes {
        if world.invalid.contains(&lane.id) || world.paused_owners.contains(&lane.owner) {
            continue;
        }
        if world.cooling_until.get(&lane.id).copied().unwrap_or(0) <= now + horizon_ms {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Default)]
pub struct RateBucket {
    pub recent: Vec<u64>,
    pub day_key: String,
    pub day_count: u64,
}

/// Per-minute + per-day probe allowance (UTC day key like TS).
pub fn rate_allow(
    bucket: &RateBucket,
    now: u64,
    per_min: u64,
    daily: u64,
    day_key: &str,
) -> (bool, RateBucket) {
    let day_count = if bucket.day_key == day_key {
        bucket.day_count
    } else {
        0
    };
    let mut recent: Vec<u64> = bucket
        .recent
        .iter()
        .copied()
        .filter(|t| now.saturating_sub(*t) < 60_000)
        .collect();
    let allow = (recent.len() as u64) < per_min && day_count < daily;
    if allow {
        recent.push(now);
    }
    (
        allow,
        RateBucket {
            recent,
            day_key: day_key.to_string(),
            day_count: if allow { day_count + 1 } else { day_count },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000_000;

    fn lanes() -> Vec<ProbeLane> {
        vec![
            ProbeLane {
                id: "dahl/MiniMaxAI/MiniMax-M2.7".into(),
                owner: "dahl".into(),
            },
            ProbeLane {
                id: "freeinference/deepseek-v4-flash".into(),
                owner: "freeinference".into(),
            },
            ProbeLane {
                id: "agentrouter/gpt-5.6-sol".into(),
                owner: "agentrouter".into(),
            },
        ]
    }

    fn world() -> ProbeWorld {
        ProbeWorld::default()
    }

    fn fact(kind: &str) -> ProbeFact {
        ProbeFact {
            kind: kind.into(),
            fail_count: 0,
            ..Default::default()
        }
    }

    #[test]
    fn backoff_ladder_and_jitter() {
        assert_eq!(probe_backoff_ms(1, 0.5), 30_000);
        assert_eq!(probe_backoff_ms(2, 0.5), 60_000);
        assert_eq!(probe_backoff_ms(3, 0.5), 120_000);
        assert_eq!(probe_backoff_ms(4, 0.5), 300_000);
        assert_eq!(probe_backoff_ms(9, 0.5), 600_000);
        assert_eq!(probe_backoff_ms(1, 0.0), 24_000);
        assert_eq!(probe_backoff_ms(1, 1.0), 36_000);
    }

    #[test]
    fn selection_priority_and_skips() {
        let mut w = world();
        w.probes.insert(
            "dahl/MiniMaxAI/MiniMax-M2.7".into(),
            ProbeFact {
                kind: "cooling".into(),
                until: Some(NOW - 5_000),
                next_probe_at: NOW,
                fail_count: 1,
                ..Default::default()
            },
        );
        w.probes.insert(
            "freeinference/deepseek-v4-flash".into(),
            ProbeFact {
                kind: "ok".into(),
                ok_at: Some(NOW - 11 * 60_000),
                next_probe_at: NOW,
                fail_count: 0,
                ..Default::default()
            },
        );
        // Unverified lane wins.
        let c = select_probe_candidate(&lanes(), &w, NOW, DEFAULT_VERIFIED_TTL_MS).unwrap();
        assert_eq!(c.lane.id, "agentrouter/gpt-5.6-sol");
        assert_eq!(c.why, "unverified");

        let mut w2 = w.clone();
        w2.probes.insert(
            "agentrouter/gpt-5.6-sol".into(),
            ProbeFact {
                kind: "ok".into(),
                ok_at: Some(NOW - 1_000),
                next_probe_at: NOW,
                ..Default::default()
            },
        );
        let c2 = select_probe_candidate(&lanes(), &w2, NOW, DEFAULT_VERIFIED_TTL_MS).unwrap();
        assert_eq!(c2.why, "cooldown-expired");

        let mut w3 = world();
        w3.invalid.insert("agentrouter/gpt-5.6-sol".into());
        w3.paused_owners.insert("freeinference".into());
        w3.cooling_until
            .insert("dahl/MiniMaxAI/MiniMax-M2.7".into(), NOW + 60_000);
        assert!(select_probe_candidate(&lanes(), &w3, NOW, DEFAULT_VERIFIED_TTL_MS).is_none());
    }

    #[test]
    fn exhaustion_needs_all_lanes_out() {
        let mut w = world();
        w.cooling_until
            .insert("dahl/MiniMaxAI/MiniMax-M2.7".into(), NOW + 60_000);
        assert!(!is_exhausted(
            &lanes(),
            &w,
            NOW,
            DEFAULT_EXHAUSTED_HORIZON_MS
        ));

        let mut all = world();
        all.invalid.insert("dahl/MiniMaxAI/MiniMax-M2.7".into());
        all.paused_owners.insert("freeinference".into());
        all.cooling_until
            .insert("agentrouter/gpt-5.6-sol".into(), NOW + 31 * 60_000);
        assert!(is_exhausted(
            &lanes(),
            &all,
            NOW,
            DEFAULT_EXHAUSTED_HORIZON_MS
        ));
        assert!(is_exhausted(
            &[],
            &world(),
            NOW,
            DEFAULT_EXHAUSTED_HORIZON_MS
        ));
    }

    #[test]
    fn rate_limits() {
        let mut bucket = RateBucket::default();
        for i in 0..3 {
            let (allow, next) = rate_allow(&bucket, NOW + i, 3, 100, "day");
            assert!(allow);
            bucket = next;
        }
        let (allow, _) = rate_allow(&bucket, NOW + 10, 3, 100, "day");
        assert!(!allow);
        let (allow, next) = rate_allow(&bucket, NOW + 86_400_000, 3, 100, "next-day");
        assert!(allow);
        assert_eq!(next.day_count, 1);
    }

    #[test]
    fn verified_ttl() {
        let mut probes = HashMap::new();
        probes.insert(
            "x".to_string(),
            ProbeFact {
                kind: "ok".into(),
                ok_at: Some(NOW - 1_000),
                ..Default::default()
            },
        );
        assert!(is_verified("x", &probes, NOW, DEFAULT_VERIFIED_TTL_MS));
        assert!(!is_verified(
            "x",
            &probes,
            NOW + 11 * 60_000,
            DEFAULT_VERIFIED_TTL_MS
        ));
        assert!(!is_verified("y", &probes, NOW, DEFAULT_VERIFIED_TTL_MS));
        let _ = fact("ok");
    }
}
