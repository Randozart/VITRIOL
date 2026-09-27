// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md, M2). Session affinity: pins a
// session id to a lane so a conversation stays put (KV/session state lives
// on the lane that served it). TTL-bounded: a stale pin expires so the
// arbiter regains control. knooppunt has nothing like this - it is
// stateless per request by design.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Binding {
    pub lane: String,
    pub pinned_at_ms: i64,
    pub last_seen_ms: i64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug)]
pub struct SessionTable {
    inner: Mutex<HashMap<String, Binding>>,
    ttl_ms: i64,
}

impl SessionTable {
    pub fn new(ttl_secs: i64) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl_ms: ttl_secs * 1000,
        }
    }

    /// Live binding for a session, or None (absent or expired).
    /// Expired entries are dropped on read (lazy sweep).
    pub fn get(&self, session: &str) -> Option<Binding> {
        let mut map = self.inner.lock().ok()?;
        let now = now_ms();
        let b = map.get(session)?;
        if now - b.last_seen_ms >= self.ttl_ms {
            map.remove(session);
            return None;
        }
        Some(b.clone())
    }

    /// Pin (or refresh) a session's lane binding.
    pub fn pin(&self, session: &str, lane: &str) {
        if let Ok(mut map) = self.inner.lock() {
            let now = now_ms();
            let pinned_at = map.get(session).map(|b| b.pinned_at_ms).unwrap_or(now);
            map.insert(
                session.to_string(),
                Binding {
                    lane: lane.to_string(),
                    pinned_at_ms: pinned_at,
                    last_seen_ms: now,
                },
            );
        }
    }

    /// Drop a session's pin only if it points at `lane`; used by the
    /// privacy gate to yank a session home without touching others.
    pub fn clear_if(&self, session: Option<&str>, lane: &str) {
        let Some(sid) = session else { return };
        if let Ok(mut map) = self.inner.lock() {
            if map.get(sid).map(|b| b.lane.as_str()) == Some(lane) {
                return; // pin already home; keep it
            }
            map.remove(sid);
        }
    }

    /// Drop a binding (lane unhealthy, manual reset).
    pub fn clear(&self, session: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(session);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drop every binding older than the TTL (periodic sweep).
    pub fn sweep(&self) {
        if let Ok(mut map) = self.inner.lock() {
            let now = now_ms();
            map.retain(|_, b| now - b.last_seen_ms < self.ttl_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_get_roundtrip() {
        let t = SessionTable::new(1800);
        t.pin("ses_a", "zen");
        assert_eq!(t.get("ses_a").unwrap().lane, "zen");
        assert!(t.get("ses_b").is_none());
    }

    #[test]
    fn re_pin_updates_lane_keeps_original_pin_time() {
        let t = SessionTable::new(1800);
        t.pin("ses_a", "zen");
        let first = t.get("ses_a").unwrap();
        t.pin("ses_a", "lapis");
        let second = t.get("ses_a").unwrap();
        assert_eq!(second.lane, "lapis");
        assert_eq!(first.pinned_at_ms, second.pinned_at_ms);
    }

    #[test]
    fn ttl_expiry_drops_binding() {
        let t = SessionTable::new(0); // everything expires immediately
        t.pin("ses_a", "zen");
        assert!(t.get("ses_a").is_none());
    }

    #[test]
    fn clear_forgets() {
        let t = SessionTable::new(1800);
        t.pin("ses_a", "zen");
        t.clear("ses_a");
        assert!(t.get("ses_a").is_none());
        assert_eq!(t.len(), 0);
    }
}
