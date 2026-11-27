//! vitriol-core — pure decision logic shared by VITRIOL/Officina routing.
//!
//! Phase 1 of the Rust-core plan (`.opencode/plans/caduceus-admission-routing-2026-11-14.md`):
//! provider-error facts, owner admission, token budgets. No I/O, no network,
//! no globals — everything here is total and unit-testable, and the same
//! golden vectors are asserted on the TypeScript side to keep the two
//! implementations honest (parity invariant I2).
//!
//! PROVENANCE: original work, this repo (logic ported from
//! `officina/.pi/extensions/_shared/gateway-health.ts`, 2026-11-14).

pub mod cache_lifetime;
pub mod caduceus;
pub mod scheduler;

/// Facts extracted from one provider error body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderFacts {
    /// "maximum context length is N tokens" / "limit L".
    pub context_limit: Option<u64>,
    /// Prompt size quoted by the provider ("value=171809",
    /// "estimated 183106 input tokens").
    pub overflow_tokens: Option<u64>,
    /// 429 credential cooldown ("reset_seconds" / "retry_after" / "reset_at").
    pub cooldown_ms: Option<u64>,
    /// Owner-wide 400 key pause ("paused for N minutes").
    pub pause_ms: Option<u64>,
    /// Degraded safe input size while paused ("under about N KB").
    pub safe_bytes: Option<u64>,
    /// 404 model_not_found: missing id + the upstream's offered alternatives.
    pub missing_model: Option<String>,
    pub alternatives: Vec<String>,
}

fn find_number(text: &str, marker: &str) -> Option<u64> {
    let idx = text.find(marker)?;
    let rest = &text[idx + marker.len()..];
    let digits: String = rest
        .trim_start_matches([':', ' ', '=', '"'])
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Parse one provider error body into facts. Never panics; unknown bodies
/// simply yield all-`None`.
pub fn parse_provider_error(text: &str) -> ProviderFacts {
    let mut facts = ProviderFacts {
        context_limit: find_number(text, "maximum context length is")
            .or_else(|| find_number(text, "limit ")),
        overflow_tokens: find_number(text, "value=")
            .or_else(|| find_number(text, "estimated "))
            .or_else(|| find_number(text, "prompt contains at least ")),
        ..ProviderFacts::default()
    };

    // Credential cooldown.
    if let Some(secs) = find_number(text, "reset_seconds") {
        facts.cooldown_ms = Some(secs.saturating_mul(1000));
    }

    // Owner-wide key pause + degraded safe size.
    if let Some(minutes) = find_number(text, "paused for ") {
        facts.pause_ms = Some(minutes.saturating_mul(60_000));
    }
    if let Some(kb) = find_number(text, "under about ") {
        // "under about 56 KB"
        if text.contains("KB") {
            facts.safe_bytes = Some(kb.saturating_mul(1024));
        }
    }

    // 404 model_not_found: missing id and offered alternatives.
    if text.contains("not currently offered") {
        facts.missing_model = text
            .find("not currently offered:")
            .map(|i| {
                text[i + "not currently offered:".len()..]
                    .trim_start_matches([' ', ':', '"'])
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != ',' && *c != '"')
                    .collect::<String>()
                    .trim_end_matches('.')
                    .to_string()
            })
            .filter(|s| !s.is_empty());
        if let Some(raw) = text.split("Available models:").nth(1) {
            let list = raw
                .split(". To switch")
                .next()
                .unwrap_or(raw)
                .split("To switch")
                .next()
                .unwrap_or(raw);
            facts.alternatives = list
                .split(',')
                .map(|s| s.trim().trim_end_matches('.').trim_matches('"').to_string())
                .filter(|s| !s.is_empty() && s.len() < 120)
                .collect();
        }
    }

    facts
}

/// Effective input window for a lane, in tokens.
///
/// `declared` is the catalog window, `learned` a provider-confirmed limit,
/// `safe_bytes` the owner's degraded ceiling while paused (bytes → tokens at
/// chars/4). The smallest non-zero value wins; `None` when all are unknown.
pub fn effective_window(
    declared: u64,
    learned: Option<u64>,
    safe_bytes: Option<u64>,
) -> Option<u64> {
    let safe_tokens = safe_bytes.map(|b| b / 4);
    let mut window = declared;
    for value in [learned, safe_tokens].into_iter().flatten() {
        if value == 0 {
            continue;
        }
        if window == 0 || value < window {
            window = value;
        }
    }
    (window > 0).then_some(window)
}

/// Admission decision for one lane (mirrors `cloudEligible` in TS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Admit,
    Invalid,
    Cooling,
    OwnerPaused,
    NoFit,
}

#[derive(Debug, Clone, Default)]
pub struct LaneState {
    pub invalid: bool,
    pub cooling: bool,
    pub owner_paused: bool,
}

pub fn admit(
    prompt_tokens: u64,
    reserve_tokens: u64,
    lane: &LaneState,
    declared: u64,
    learned: Option<u64>,
    safe_bytes: Option<u64>,
) -> Admission {
    if lane.invalid {
        return Admission::Invalid;
    }
    if lane.cooling {
        return Admission::Cooling;
    }
    if lane.owner_paused {
        return Admission::OwnerPaused;
    }
    match effective_window(declared, learned, safe_bytes) {
        Some(window) if prompt_tokens.saturating_add(reserve_tokens) < window => Admission::Admit,
        _ => Admission::NoFit,
    }
}

/// Billed-token total: zero-cost lanes never count (operator decision).
pub fn billed_tokens(rows: &[(f64, u64)]) -> u64 {
    rows.iter()
        .filter(|(cost, _)| *cost > 0.0)
        .map(|(_, tokens)| *tokens)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The exact production bodies this logic exists to kill.
    const OVERFLOW_180: &str = "[400]: This model's maximum context length is 180000 tokens. However, you requested 8192 output tokens and your prompt contains at least 171809 input tokens, for a total of at least 180001 tokens. (parameter=input_tokens, value=171809)";
    const PAUSE_400: &str = "[400]: This model's maximum context length is 180000 tokens, and this request may not fit. Several requests from this key went over the limit, so requests that may not fit are paused for 11 minutes. Requests under about 56 KB still go through";
    const NOT_FOUND_404: &str = "[404]: This model is not currently offered: moonshotai/Kimi-K2.6. Available models: MiniMaxAI/MiniMax-M2.7, deepseek-ai/DeepSeek-V4-Flash-0731, zai-org/GLM-5.3-Flash. To switch, set \"model\" in your request to one of these IDs exactly as written.";
    const COOLDOWN_429: &str = "429: {\"message\":\"All credentials for model deepseek-v4-flash are cooling down\",\"type\":\"rate_limit_error\",\"code\":\"model_cooldown\",\"model\":\"deepseek-v4-flash\",\"reset_seconds\":20054,\"retry_after\":\"2026-11-14T23:59:59.660Z\"}";
    const GATEWAY_LIMIT: &str = "400: {\"message\":\"Input exceeds context window for freeinference/qwen3.6-35b: estimated 183106 input tokens, limit 128000. Reduce the prompt or route to a model with a larger context window.\"}";

    #[test]
    fn parses_the_180k_overflow() {
        let f = parse_provider_error(OVERFLOW_180);
        assert_eq!(f.context_limit, Some(180_000));
        assert_eq!(f.overflow_tokens, Some(171_809));
        assert_eq!(f.pause_ms, None);
    }

    #[test]
    fn parses_the_owner_pause_and_safe_bytes() {
        let f = parse_provider_error(PAUSE_400);
        assert_eq!(f.context_limit, Some(180_000));
        assert_eq!(f.pause_ms, Some(11 * 60_000));
        assert_eq!(f.safe_bytes, Some(56 * 1024));
    }

    #[test]
    fn parses_the_404_alternatives() {
        let f = parse_provider_error(NOT_FOUND_404);
        assert_eq!(f.missing_model.as_deref(), Some("moonshotai/Kimi-K2.6"));
        assert_eq!(
            f.alternatives,
            vec![
                "MiniMaxAI/MiniMax-M2.7",
                "deepseek-ai/DeepSeek-V4-Flash-0731",
                "zai-org/GLM-5.3-Flash"
            ]
        );
    }

    #[test]
    fn parses_cooldown_and_gateway_limit() {
        assert_eq!(
            parse_provider_error(COOLDOWN_429).cooldown_ms,
            Some(20_054_000)
        );
        let g = parse_provider_error(GATEWAY_LIMIT);
        assert_eq!(g.context_limit, Some(128_000));
        assert_eq!(g.overflow_tokens, Some(183_106));
    }

    #[test]
    fn effective_window_takes_the_smallest_known_value() {
        assert_eq!(
            effective_window(400_000, Some(180_000), None),
            Some(180_000)
        );
        assert_eq!(
            effective_window(400_000, None, Some(56 * 1024)),
            Some(14_336)
        );
        assert_eq!(effective_window(262_144, None, None), Some(262_144));
        assert_eq!(effective_window(0, None, None), None);
    }

    #[test]
    fn admission_matrix() {
        let ok = LaneState::default();
        assert_eq!(
            admit(100_000, 16_384, &ok, 180_000, None, None),
            Admission::Admit
        );
        assert_eq!(
            admit(170_000, 16_384, &ok, 180_000, None, None),
            Admission::NoFit
        );
        assert_eq!(
            admit(
                100_000,
                16_384,
                &LaneState {
                    invalid: true,
                    ..ok.clone()
                },
                180_000,
                None,
                None
            ),
            Admission::Invalid
        );
        assert_eq!(
            admit(
                100_000,
                16_384,
                &LaneState {
                    owner_paused: true,
                    ..ok.clone()
                },
                180_000,
                None,
                None
            ),
            Admission::OwnerPaused
        );
        assert_eq!(
            admit(20_000, 16_384, &ok, 180_000, None, Some(56 * 1024)),
            Admission::NoFit
        );
    }

    #[test]
    fn billed_only_budget() {
        assert_eq!(
            billed_tokens(&[(0.0, 9_000_000), (0.002, 50), (0.01, 100)]),
            150
        );
    }
}
