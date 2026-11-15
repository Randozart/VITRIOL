// Harness self-healing policy (2026-11-15).
//
// The TUI already aborts a stalled tool and releases the UI. This module
// supplies the bounded decision layer above that: classify the failure,
// decide whether a recovery attempt is still in budget, and identify the
// one environment action that is allowed (engine start ONLY under
// officina/lapis, model switch to lapis, or notice-only).

use std::time::{Duration, Instant};

use crate::rpc::protocol::Model;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    EngineDown,
    NoPhysicalModel,
    ModelNotFound,
    Auth,
    RateLimit,
    ContextOverflow,
    ToolStall,
    WireIdle,
    ChildExit,
    AbortUnsettled,
    Unknown,
}

/// Classify a provider/model error string. Order matters: auth and quota
/// errors are actionable messages, not engine outages.
pub fn classify_error(text: &str) -> Failure {
    let s = text.to_ascii_lowercase();
    if s.contains("no api key")
        || s.contains("invalid api key")
        || s.contains("unauthorized")
        || s.contains("authentication")
        || s.contains("401")
        || s.contains("403")
    {
        return Failure::Auth;
    }
    if s.contains("429")
        || s.contains("rate limit")
        || s.contains("too many requests")
        || s.contains("cooling")
        || s.contains("cooldown")
    {
        return Failure::RateLimit;
    }
    if s.contains("context length")
        || s.contains("context window")
        || s.contains("context overflow")
        || s.contains("too many tokens")
        || s.contains("exceeds the model")
    {
        return Failure::ContextOverflow;
    }
    if s.contains("no physical model") {
        return Failure::NoPhysicalModel;
    }
    if s.contains("model not found")
        || s.contains("no such model")
        || s.contains("unknown model")
        || s.contains("invalid model")
        || s.contains("not offered")
        || s.contains("model does not exist")
    {
        return Failure::ModelNotFound;
    }
    if s.contains("engine down")
        || s.contains("engine unavailable")
        || s.contains("connection refused")
        || s.contains("econnrefused")
        || s.contains("failed to fetch")
        || s.contains("fetch failed")
        || s.contains("socket hang up")
        || s.contains("network error")
        || s.contains("connect etimedout")
        || s.contains("503")
        || s.contains("502")
        || s.contains("504")
    {
        return Failure::EngineDown;
    }
    Failure::Unknown
}

/// Only the virtual lapis model may auto-start/restart the engine.
pub fn is_lapis(model: &Option<Model>) -> bool {
    model
        .as_ref()
        .is_some_and(|m| m.provider == "officina" && m.id == "lapis")
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => default,
    }
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub max_attempts: usize,
    pub window: Duration,
    pub backoff: Vec<Duration>,
    pub wire_idle: Duration,
    /// One bounded grace before a silent tool is aborted (2026-11-15).
    pub stall_grace: Duration,
    pub replay: bool,
    pub engine_start: bool,
    pub engine_restart: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            window: Duration::from_secs(600),
            backoff: vec![
                Duration::from_secs(1),
                Duration::from_secs(5),
                Duration::from_secs(15),
            ],
            wire_idle: Duration::from_secs(300),
            stall_grace: Duration::from_secs(120),
            replay: true,
            engine_start: true,
            engine_restart: false,
        }
    }
}

impl Policy {
    pub fn from_env() -> Self {
        let mut p = Self::default();
        p.max_attempts = env_u64("OFFICINA_HEAL_MAX_ATTEMPTS", p.max_attempts as u64) as usize;
        p.window = Duration::from_secs(env_u64("OFFICINA_HEAL_WINDOW_SECS", p.window.as_secs()));
        p.wire_idle =
            Duration::from_secs(env_u64("OFFICINA_WIRE_IDLE_SECS", p.wire_idle.as_secs()));
        p.stall_grace = Duration::from_secs(env_u64(
            "OFFICINA_STALL_GRACE_SECS",
            p.stall_grace.as_secs(),
        ));
        p.replay = env_bool("OFFICINA_HEAL_REPLAY", p.replay);
        p.engine_start = env_bool("OFFICINA_HEAL_ENGINE", p.engine_start);
        p.engine_restart = env_bool("OFFICINA_HEAL_ENGINE_RESTART", p.engine_restart);
        p
    }

    pub fn backoff_for(&self, attempt: usize) -> Duration {
        self.backoff
            .get(attempt.saturating_sub(1))
            .copied()
            .unwrap_or_else(|| {
                self.backoff
                    .last()
                    .copied()
                    .unwrap_or(Duration::from_secs(5))
            })
    }
}

/// Master kill switch. Default ON; OFFICINA_SELF_HEAL=0 restores the old
/// abort-and-release behavior byte for byte (I1).
pub fn self_heal_enabled() -> bool {
    env_bool("OFFICINA_SELF_HEAL", true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Aborting,
    Respawning,
    Rehydrating,
    Degraded,
}

#[derive(Debug, Clone)]
pub struct PendingReplay {
    pub at: Instant,
    /// True when the replay must wait for the selected model to become
    /// officina/lapis (model switch / engine start). False after a respawn.
    pub requires_lapis: bool,
}

#[derive(Debug)]
pub struct RecoveryState {
    pub phase: Phase,
    /// Attempt instants inside the sliding window.
    pub attempts: Vec<Instant>,
    pub reason: String,
    pub last_error: Option<String>,
    pub degraded: bool,
    pub pending_replay: Option<PendingReplay>,
    pub engine_deadline: Option<Instant>,
    /// Throttle for the engine readiness probe while a start is pending.
    pub engine_next_probe: Option<Instant>,
    pub rehydrate_deadline: Option<Instant>,
    /// Set when get_messages has been applied, so a late `session_start`
    /// cannot wipe the restored transcript.
    pub rehydrate_grace_until: Option<Instant>,
    pub next_retry_at: Option<Instant>,
    /// Last classified provider failure + when, for dedupe.
    pub last_failure: Option<(Failure, Instant)>,
    /// Failure handed from handle_event (sync) to the async run loop.
    pub pending_failure: Option<(Failure, String)>,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            attempts: Vec::new(),
            reason: String::new(),
            last_error: None,
            degraded: false,
            pending_replay: None,
            engine_deadline: None,
            engine_next_probe: None,
            rehydrate_deadline: None,
            rehydrate_grace_until: None,
            next_retry_at: None,
            last_failure: None,
            pending_failure: None,
        }
    }
}

impl RecoveryState {
    pub fn can_attempt(&self, now: Instant, policy: &Policy) -> bool {
        let recent = self
            .attempts
            .iter()
            .filter(|t| now.duration_since(**t) <= policy.window)
            .count();
        recent < policy.max_attempts
    }

    pub fn record_attempt(&mut self, now: Instant, reason: &str) {
        self.attempts.push(now);
        self.reason = reason.to_string();
        self.phase = Phase::Respawning;
        self.next_retry_at = None;
    }

    pub fn degrade(&mut self, reason: &str) {
        self.phase = Phase::Degraded;
        self.degraded = true;
        self.reason = reason.to_string();
        self.next_retry_at = None;
        self.rehydrate_deadline = None;
        self.engine_deadline = None;
        self.engine_next_probe = None;
    }

    pub fn recovered(&mut self) {
        self.phase = Phase::Idle;
        self.degraded = false;
        self.rehydrate_deadline = None;
        self.next_retry_at = None;
        self.last_error = None;
        self.rehydrate_grace_until = Some(Instant::now() + Duration::from_secs(5));
    }

    pub fn failure_seen(&mut self, failure: Failure, now: Instant) -> bool {
        let duplicate = self.last_failure.is_some_and(|(last, at)| {
            last == failure && now.duration_since(at) < Duration::from_secs(60)
        });
        self.last_failure = Some((failure, now));
        duplicate
    }
}

/// Evidence collected before a wire-idle escalation. Escalate only when
/// both the wire and pi's CPU have been flat; a busy engine extends the
/// deadline instead of killing a legitimate long prefill.
#[derive(Debug, Clone, Copy)]
pub struct WireEvidence {
    pub idle: Duration,
    pub pi_flat: Duration,
    pub engine_busy: bool,
}

pub fn wire_idle_escalate(threshold: Duration, ev: WireEvidence) -> bool {
    threshold > Duration::ZERO
        && ev.idle >= threshold
        && ev.pi_flat >= threshold.min(ev.idle)
        && !ev.engine_busy
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineActivity {
    UpBusy,
    UpIdle,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineEndpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn engine_url() -> String {
    std::env::var("OFFICINA_ENGINE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8279".to_string())
}

/// Parse the engine base URL into a /slots probe target. `None` for
/// unsupported/malformed URLs (the caller treats that as "down/unknown").
pub fn parse_engine_url(url: &str) -> Option<EngineEndpoint> {
    let rest = url
        .trim()
        .strip_prefix("http://")
        .or_else(|| url.trim().strip_prefix("https://"))?;
    let (authority, base_path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, String::new()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (authority, 80),
    };
    if host.is_empty() {
        return None;
    }
    let base = base_path.trim_end_matches('/');
    Some(EngineEndpoint {
        host: host.to_string(),
        port,
        path: format!("{base}/slots"),
    })
}

/// Non-blocking-ish engine liveness probe used only after the wire has
/// already gone quiet. A tiny HTTP/1.1 GET keeps the TUI dependency-free.
pub async fn engine_activity() -> EngineActivity {
    let Some(ep) = parse_engine_url(&engine_url()) else {
        return EngineActivity::Down;
    };
    let probe = async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect((ep.host.as_str(), ep.port)).await?;
        let req = format!(
            "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            ep.path, ep.host, ep.port
        );
        stream.write_all(req.as_bytes()).await?;
        let mut body = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    body.extend_from_slice(&chunk[..n]);
                    if body.len() >= 64 * 1024 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        Ok::<String, std::io::Error>(String::from_utf8_lossy(&body).into_owned())
    };
    match tokio::time::timeout(Duration::from_secs(2), probe).await {
        Ok(Ok(body)) => {
            let busy = body.contains("\"is_processing\":true")
                || body.contains("\"is_processing\": true")
                || body.contains("\"is_processing\":  true");
            if busy {
                EngineActivity::UpBusy
            } else {
                EngineActivity::UpIdle
            }
        }
        _ => EngineActivity::Down,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_provider_failures_in_priority_order() {
        assert_eq!(
            classify_error("Lapis Occultus: no physical model available — engine down"),
            Failure::NoPhysicalModel
        );
        assert_eq!(classify_error("engine unavailable"), Failure::EngineDown);
        assert_eq!(
            classify_error("No API key found for provider"),
            Failure::Auth
        );
        assert_eq!(classify_error("429 retry_after"), Failure::RateLimit);
        assert_eq!(
            classify_error("context length exceeded"),
            Failure::ContextOverflow
        );
        assert_eq!(
            classify_error("model not found: caduceus/x"),
            Failure::ModelNotFound
        );
        assert_eq!(classify_error("something else"), Failure::Unknown);
    }

    #[test]
    fn only_lapis_may_start_the_engine() {
        let lapis = Model {
            id: "lapis".into(),
            name: None,
            provider: "officina".into(),
            context_window: None,
            max_tokens: None,
            reasoning: None,
        };
        let local = Model {
            id: "Lapis Occultus".into(),
            provider: "llamacpp".into(),
            ..lapis.clone()
        };
        assert!(is_lapis(&Some(lapis)));
        assert!(!is_lapis(&Some(local)));
        assert!(!is_lapis(&None));
    }

    #[test]
    fn policy_is_bounded_and_backs_off() {
        let p = Policy {
            max_attempts: 3,
            window: Duration::from_secs(60),
            ..Policy::default()
        };
        let now = Instant::now();
        let mut r = RecoveryState::default();
        assert!(r.can_attempt(now, &p));
        r.record_attempt(now, "a");
        assert_eq!(r.phase, Phase::Respawning);
        assert_eq!(p.backoff_for(1), Duration::from_secs(1));
        assert_eq!(p.backoff_for(2), Duration::from_secs(5));
        assert_eq!(p.backoff_for(3), Duration::from_secs(15));
        assert_eq!(p.backoff_for(9), Duration::from_secs(15));
        r.record_attempt(now, "b");
        r.record_attempt(now, "c");
        assert!(!r.can_attempt(now, &p), "budget exhausted inside window");
        // Outside the window the old attempts no longer count.
        assert!(r.can_attempt(now + Duration::from_secs(61), &p));
    }

    #[test]
    fn wire_idle_needs_flat_pi_and_idle_engine() {
        let t = Duration::from_secs(300);
        assert!(wire_idle_escalate(
            t,
            WireEvidence {
                idle: t,
                pi_flat: t,
                engine_busy: false
            }
        ));
        assert!(!wire_idle_escalate(
            t,
            WireEvidence {
                idle: t,
                pi_flat: Duration::from_secs(10),
                engine_busy: false
            }
        ));
        assert!(!wire_idle_escalate(
            t,
            WireEvidence {
                idle: t,
                pi_flat: t,
                engine_busy: true
            }
        ));
        assert!(!wire_idle_escalate(
            Duration::ZERO,
            WireEvidence {
                idle: t,
                pi_flat: t,
                engine_busy: false
            }
        ));
    }

    #[test]
    fn engine_url_parser_targets_slots() {
        assert_eq!(
            parse_engine_url("http://127.0.0.1:8279"),
            Some(EngineEndpoint {
                host: "127.0.0.1".into(),
                port: 8279,
                path: "/slots".into()
            })
        );
        assert_eq!(
            parse_engine_url("http://host:9000/v1/"),
            Some(EngineEndpoint {
                host: "host".into(),
                port: 9000,
                path: "/v1/slots".into()
            })
        );
        assert_eq!(parse_engine_url("file:///tmp/x"), None);
    }

    #[test]
    fn duplicate_failures_are_suppressed_for_a_minute() {
        let now = Instant::now();
        let mut r = RecoveryState::default();
        assert!(!r.failure_seen(Failure::EngineDown, now));
        assert!(r.failure_seen(Failure::EngineDown, now + Duration::from_secs(10)));
        assert!(!r.failure_seen(Failure::EngineDown, now + Duration::from_secs(71)));
    }
}
