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
    /// Named variant label; empty for a matrix arm (its label is generated).
    pub name: String,
    pub flags: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
    /// Verbatim flag words for a named variant (`--spec-type draft-mtp` etc.).
    ///
    /// A knob that needs *two* flags at once (spec-type + draft depth) cannot be
    /// expressed as a one-dimensional `--set`, and crossing two `--set`s would
    /// sweep combinations nobody asked for. A variant states the words.
    pub raw: Vec<String>,
}

impl Arm {
    /// Human label: `score_every=8 · VITRIOL_KV_SCORE=probe`.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        let mut parts: Vec<String> = self.flags.iter().map(|(k, v)| format!("{k}={v}")).collect();
        parts.extend(self.env.iter().map(|(k, v)| format!("{k}={v}")));
        if !self.raw.is_empty() {
            parts.push(self.raw.join(" "));
        }
        parts.join(" · ")
    }

    /// Full argv for one start: `base` split on whitespace, then the arm's flags.
    pub fn argv(&self, base: &str, engine: &str) -> Vec<String> {
        let mut argv = vec![engine.to_string()];
        argv.extend(base.split_whitespace().map(|s| s.to_string()));
        argv.extend(self.raw.iter().cloned());
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

/// Named variants: `label=--flag val --flag2 val2`, repeatable.
///
/// An empty flag string is legitimate — it is the "no speculative decoding" arm,
/// and the probe that proves the label is honoured rather than ignored.
pub fn variants(specs: &[String]) -> Vec<Arm> {
    specs
        .iter()
        .filter_map(|spec| {
            let (label, flags) = spec.split_once('=')?;
            if label.trim().is_empty() {
                return None;
            }
            Some(Arm {
                name: label.trim().to_string(),
                flags: Vec::new(),
                env: Vec::new(),
                raw: flags.split_whitespace().map(|w| w.to_string()).collect(),
            })
        })
        .collect()
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
    fn a_variant_carries_verbatim_flag_words() {
        let arms = variants(&["mtp2=--spec-type mtp --spec-draft-n-max 2".to_string()]);
        assert_eq!(arms.len(), 1);
        assert_eq!(arms[0].label(), "mtp2");
        assert_eq!(
            arms[0].argv("--model /m.gguf", "/bin/s"),
            vec![
                "/bin/s",
                "--model",
                "/m.gguf",
                "--spec-type",
                "mtp",
                "--spec-draft-n-max",
                "2",
            ]
        );
    }

    #[test]
    fn a_variant_may_carry_no_flags() {
        // The "off" arm is a real arm: it must start, not vanish.
        let arms = variants(&["none=".to_string()]);
        assert_eq!(arms.len(), 1);
        assert!(arms[0].raw.is_empty());
        assert_eq!(arms[0].argv("--x", "/bin/s"), vec!["/bin/s", "--x"]);
    }

    #[test]
    fn argv_keeps_the_base_and_appends_arm_flags() {
        let arm = Arm {
            name: String::new(),
            flags: vec![("score_every".into(), "8".into())],
            env: vec![],
            raw: vec![],
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
