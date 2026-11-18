// Sweep: run one engine per configuration and compare (E6 — eviction tuning).
//
// E6 asks whether `score_every` / `n_sinks` are set well, which can only be
// answered by starting the engine with different flags and re-measuring the same
// shapes. Doing that by hand invites the classic mistake this repo keeps
// recording: a config drifts between arms and the comparison is worthless.
//
// So the matrix is generated, printed (`--dry-run`) and fingerprinted:
//
//   density-bench sweep --engine /path/llama-server \
//       --arg "--model /models/x.gguf --port 8287 -ngl 99" \
//       --set "score_every=8,16,32" --env "VITRIOL_KV_SCORE=probe" \
//       --shapes deep-recall --repeats 2
//
// `--set k=v1,v2` becomes `--k v` (a CLI flag); `--env k=v1,v2` becomes an env
// var. Both are cartesian, and every arm's argv+env is printed before anything
// starts, so a typo is visible before it costs an engine load.

/// One configuration arm: the extra flags and env for a single engine start.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Arm {
    pub flags: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
}

impl Arm {
    /// Human label: `score_every=8 · VITRIOL_KV_SCORE=probe`.
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = self.flags.iter().map(|(k, v)| format!("{k}={v}")).collect();
        parts.extend(self.env.iter().map(|(k, v)| format!("{k}={v}")));
        parts.join(" · ")
    }

    /// Full argv for one start: `base` split on whitespace, then the arm's flags.
    pub fn argv(&self, base: &str, engine: &str) -> Vec<String> {
        let mut argv = vec![engine.to_string()];
        argv.extend(base.split_whitespace().map(|s| s.to_string()));
        for (k, v) in &self.flags {
            argv.push(format!("--{k}"));
            argv.push(v.clone());
        }
        argv
    }
}

/// Parse `"k=v1,v2"` into a dimension. Empty/malformed entries are ignored so a
/// stray comma cannot start an engine with no flags.
pub fn dimension(spec: &str) -> Vec<(String, String)> {
    let Some((key, values)) = spec.split_once('=') else {
        return Vec::new();
    };
    values
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| (key.to_string(), v.to_string()))
        .collect()
}

/// Cartesian product of flag dimensions and env dimensions.
pub fn matrix(flag_specs: &[String], env_specs: &[String]) -> Vec<Arm> {
    let mut arms = vec![Arm::default()];
    for spec in flag_specs {
        let dim = dimension(spec);
        if dim.is_empty() {
            continue;
        }
        let mut next = Vec::new();
        for arm in &arms {
            for (k, v) in &dim {
                let mut a = arm.clone();
                a.flags.push((k.clone(), v.clone()));
                next.push(a);
            }
        }
        arms = next;
    }
    for spec in env_specs {
        let dim = dimension(spec);
        if dim.is_empty() {
            continue;
        }
        let mut next = Vec::new();
        for arm in &arms {
            for (k, v) in &dim {
                let mut a = arm.clone();
                a.env.push((k.clone(), v.clone()));
                next.push(a);
            }
        }
        arms = next;
    }
    arms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dimension_yields_one_entry_per_value() {
        assert_eq!(
            dimension("score_every=8,16,32"),
            vec![
                ("score_every".to_string(), "8".to_string()),
                ("score_every".to_string(), "16".to_string()),
                ("score_every".to_string(), "32".to_string()),
            ]
        );
    }

    #[test]
    fn malformed_specs_are_ignored_rather_than_producing_an_engine_start() {
        // A typo must not silently sweep one arm with no flags at all.
        assert!(dimension("score_every").is_empty());
        assert!(dimension("score_every=,,,").is_empty());
        assert_eq!(matrix(&["score_every".to_string()], &[]).len(), 1);
    }

    #[test]
    fn matrix_is_the_cartesian_product_and_the_first_arm_is_empty() {
        let arms = matrix(
            &["score_every=8,32".to_string()],
            &["VITRIOL_KV_SCORE=probe,off".to_string()],
        );
        assert_eq!(arms.len(), 4);
        let labels: Vec<String> = arms.iter().map(|a| a.label()).collect();
        assert!(labels.contains(&"score_every=8 · VITRIOL_KV_SCORE=probe".to_string()));
        assert!(labels.contains(&"score_every=32 · VITRIOL_KV_SCORE=off".to_string()));
    }

    #[test]
    fn argv_keeps_the_base_and_appends_arm_flags() {
        let arm = Arm {
            flags: vec![("score_every".into(), "8".into())],
            env: vec![],
        };
        let argv = arm.argv("--model /m.gguf --port 8287", "/bin/llama-server");
        assert_eq!(
            argv,
            vec![
                "/bin/llama-server",
                "--model",
                "/m.gguf",
                "--port",
                "8287",
                "--score_every",
                "8",
            ]
        );
    }
}
