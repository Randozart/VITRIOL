// PROVENANCE: inspiration — none; replay delegates directly to VITRIOL's existing governor FSM.
use serde::{Deserialize, Serialize};
use vitriol_governor::{
    config::Config,
    policy::{Action, Policy, Sample},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tick {
    pub at_secs: i64,
    pub free_mib_per_gpu: Vec<u64>,
    pub foreign_tenant: bool,
    pub engine_up: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub version: u32,
    pub provenance: crate::model::Provenance,
    #[serde(default)]
    pub policy: Policy,
    pub config: Config,
    pub ticks: Vec<Tick>,
}

#[derive(Serialize)]
pub struct Decision {
    pub at_secs: i64,
    pub state: Policy,
    pub intended_actions: Vec<serde_json::Value>,
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub evidence: &'static str,
    pub executed_actions: u64,
    pub input_sha256: String,
    pub decisions: Vec<Decision>,
}

fn action(action: Action) -> serde_json::Value {
    use serde_json::json;
    match action {
        Action::Relieve(floor) => json!({"kind":"relieve","floor":floor}),
        Action::ResizeTo(n) => json!({"kind":"resize","n_ctx":n}),
        Action::Yield => json!({"kind":"yield"}),
        Action::RelieveUndo => json!({"kind":"relieve_undo"}),
        Action::ResizeUndo(n) => json!({"kind":"resize_undo","n_ctx":n}),
        Action::Restore => json!({"kind":"restore"}),
        Action::Note(text) => json!({"kind":"note","text":text}),
    }
}

fn validate(scenario: &Scenario) -> Result<(), String> {
    if scenario.version != 1
        || scenario.ticks.is_empty()
        || scenario.ticks.len() > crate::model::MAX_EVENTS
    {
        return Err("unsupported governor version or tick count".into());
    }
    scenario.provenance.validate()?;
    validate_initial(scenario)?;
    validate_config(&scenario.config)?;
    validate_timeline(&scenario.ticks)
}

fn validate_initial(scenario: &Scenario) -> Result<(), String> {
    let first = scenario.ticks[0].at_secs;
    let timestamps = [
        scenario.policy.press_since,
        scenario.policy.clear_since,
        Some(scenario.policy.last_transition),
    ];
    if timestamps.into_iter().flatten().any(|t| t < 0 || t > first) {
        return Err(
            "initial policy timestamps must be nonnegative and no later than the first tick".into(),
        );
    }
    Ok(())
}

fn validate_config(config: &Config) -> Result<(), String> {
    if !config.relieve_floor.is_finite() || !(0.0..=1.0).contains(&config.relieve_floor) {
        return Err("relieve_floor must be finite and within 0..1".into());
    }
    let holds = [
        &config.confirm_secs,
        &config.resize_hold_secs,
        &config.yield_hold_secs,
        &config.recover_secs,
        &config.min_dwell_secs,
    ];
    if holds.iter().any(|&&v| v > i32::MAX as u64) {
        return Err("hold times exceed experiment bound".into());
    }
    Ok(())
}

fn validate_timeline(ticks: &[Tick]) -> Result<(), String> {
    let mut previous = -1;
    for tick in ticks {
        if tick.at_secs < 0
            || tick.at_secs > i32::MAX as i64
            || tick.at_secs < previous
            || tick.free_mib_per_gpu.len() > 16
        {
            return Err(
                "ticks require bounded nonnegative nondecreasing time and <=16 GPUs".into(),
            );
        }
        previous = tick.at_secs;
    }
    Ok(())
}

pub fn replay(scenario: &Scenario, input_sha256: String) -> Result<Report, String> {
    validate(scenario)?;
    let mut policy = scenario.policy.clone();
    let mut decisions = Vec::new();
    for tick in &scenario.ticks {
        let sample = Sample {
            free_mib_per_gpu: tick.free_mib_per_gpu.clone(),
            foreign_tenant: tick.foreign_tenant,
            engine_up: tick.engine_up,
        };
        let actions = policy.decide(&sample, &scenario.config, tick.at_secs);
        decisions.push(Decision {
            at_secs: tick.at_secs,
            state: policy.clone(),
            intended_actions: actions.into_iter().map(action).collect(),
        });
    }
    Ok(Report {
        version: 1,
        evidence: "governor_policy_replay",
        executed_actions: 0,
        input_sha256,
        decisions,
    })
}
