// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). Lane registry: the configured
// endpoint list is the owner's addition - Demiurge routes only across
// explicitly declared lanes; there is no discovery. VITRIOL's own engine
// is a first-class entry (always preferred).

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaneKind {
    Local,
    Node,
    #[serde(rename = "cloud-free")]
    CloudFree,
    #[serde(rename = "cloud-paid")]
    CloudPaid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub id: String,
    pub kind: LaneKind,
    pub endpoint: String,
    #[serde(default)]
    pub always_preferred: bool,
    /// engine fingerprint of the fork+model this lane serves (node lanes)
    #[serde(default)]
    pub fingerprint: Option<String>,
    /// node lane can accept spilled KV state for the declared fingerprint
    #[serde(default)]
    pub kv: bool,
    /// model ids this lane may serve (cloud lanes)
    #[serde(default)]
    pub models: Vec<String>,
    /// budget authority for paid forwards (e.g. "ascensusd")
    #[serde(default)]
    pub budget: Option<String>,
    /// arbiter-eligible overflow target (M2: cloud-free; M3/M4: node+paid)
    #[serde(default)]
    pub overflow: bool,
    /// env var name holding this lane's API key (cloud lanes)
    #[serde(default)]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub lanes: Vec<Lane>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid registry: {0}")]
    Invalid(String),
}

impl Registry {
    /// Parse + validate from a JSON string (unit-testable core).
    pub fn parse(json: &str) -> Result<Registry, ConfigError> {
        let reg: Registry = serde_json::from_str(json)?;
        reg.validate()?;
        Ok(reg)
    }

    /// Load from DEMIURGE_LANES_FILE or the default
    /// ~/.vitriol/demiurge/lanes.json.
    pub fn load() -> Result<Registry, ConfigError> {
        let path = Self::default_path();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError::Invalid(format!("cannot read {}: {e}", path.display())))?;
        Self::parse(&raw)
    }

    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("DEMIURGE_LANES_FILE") {
            return PathBuf::from(p);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        PathBuf::from(home).join(".vitriol/demiurge/lanes.json")
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.lanes.is_empty() {
            return Err(ConfigError::Invalid("no lanes declared".into()));
        }

        let mut seen = std::collections::HashSet::new();
        for lane in &self.lanes {
            if lane.id.is_empty() {
                return Err(ConfigError::Invalid("lane with empty id".into()));
            }
            if !seen.insert(lane.id.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate lane id '{}'",
                    lane.id
                )));
            }
            if !lane.endpoint.starts_with("http://") && !lane.endpoint.starts_with("https://") {
                return Err(ConfigError::Invalid(format!(
                    "lane '{}': endpoint must be http(s)",
                    lane.id
                )));
            }
            if lane.kind == LaneKind::CloudPaid && lane.budget.is_none() {
                return Err(ConfigError::Invalid(format!(
                    "lane '{}': cloud-paid requires a budget authority",
                    lane.id
                )));
            }
        }

        let preferred: Vec<_> = self.lanes.iter().filter(|l| l.always_preferred).collect();
        match preferred.len() {
            0 => Err(ConfigError::Invalid(
                "no always_preferred lane (the local engine)".into(),
            )),
            1 => {
                if preferred[0].kind != LaneKind::Local {
                    return Err(ConfigError::Invalid(format!(
                        "always_preferred lane '{}' must be kind local",
                        preferred[0].id
                    )));
                }
                Ok(())
            }
            n => Err(ConfigError::Invalid(format!(
                "{n} always_preferred lanes, exactly one allowed"
            ))),
        }
    }

    pub fn local(&self) -> &Lane {
        self.lanes
            .iter()
            .find(|l| l.always_preferred)
            .expect("validated registry has exactly one preferred lane")
    }

    pub fn get(&self, id: &str) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
        "lanes": [
            { "id": "lapis", "kind": "local", "endpoint": "http://127.0.0.1:8279", "always_preferred": true },
            { "id": "boxb", "kind": "node", "endpoint": "http://100.92.76.67:8280",
              "fingerprint": "Qwen3.8-27B", "kv": true },
            { "id": "zen", "kind": "cloud-free", "endpoint": "https://opencode.ai/zen/v1",
              "models": ["space-bunny-free"] },
            { "id": "ascensus", "kind": "cloud-paid", "endpoint": "https://api.z.ai/v4",
              "budget": "ascensusd" }
        ]
    }"#;

    #[test]
    fn parses_valid_registry() {
        let reg = Registry::parse(GOOD).expect("valid");
        assert_eq!(reg.lanes.len(), 4);
        assert_eq!(reg.local().id, "lapis");
        assert!(reg.get("boxb").unwrap().kv);
        assert_eq!(
            reg.get("zen").unwrap().models,
            vec!["space-bunny-free".to_string()]
        );
    }

    #[test]
    fn rejects_duplicate_ids() {
        let bad = GOOD.replace(
            "\"boxb\", \"kind\": \"node\"",
            "\"lapis\", \"kind\": \"node\"",
        );
        let bad = bad.replace("http://100.92.76.67:8280", "http://100.92.76.67:8281");
        assert!(Registry::parse(&bad).is_err());
    }

    #[test]
    fn rejects_missing_preferred() {
        let bad = GOOD.replace(", \"always_preferred\": true", "");
        let err = Registry::parse(&bad).expect_err("must fail");
        assert!(err.to_string().contains("always_preferred"));
    }

    #[test]
    fn rejects_two_preferred() {
        let bad = GOOD.replace(
            "\"kind\": \"node\"",
            "\"kind\": \"node\", \"always_preferred\": true",
        );
        assert!(Registry::parse(&bad).is_err());
    }

    #[test]
    fn rejects_paid_without_budget() {
        let mut val: serde_json::Value = serde_json::from_str(GOOD).unwrap();
        for lane in val["lanes"].as_array_mut().unwrap() {
            if lane["kind"] == "cloud-paid" {
                lane.as_object_mut().unwrap().remove("budget");
            }
        }
        let bad = serde_json::to_string(&val).unwrap();
        let err = Registry::parse(&bad).expect_err("must fail");
        assert!(err.to_string().contains("budget"));
    }

    #[test]
    fn rejects_non_http_endpoint() {
        let bad = GOOD.replace("http://100.92.76.67:8280", "ftp://boxb:8280");
        assert!(Registry::parse(&bad).is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        let bad = GOOD.replace("\"kind\": \"local\"", "\"kind\": \"local\", \"sneaky\": 1");
        assert!(Registry::parse(&bad).is_err());
    }
}
