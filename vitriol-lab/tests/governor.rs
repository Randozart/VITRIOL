use vitriol_governor::policy::Sample;
use vitriol_lab::governor::{self, Scenario};

fn fixture() -> Scenario {
    serde_json::from_str(include_str!("../examples/pressure.json")).unwrap()
}

#[test]
fn recorded_decisions_match_the_actual_governor_without_execution() {
    let scenario = fixture();
    let mut policy = scenario.policy.clone();
    let replay = governor::replay(&scenario, "fixture-sha".into()).unwrap();
    for (tick, result) in scenario.ticks.iter().zip(&replay.decisions) {
        let sample = Sample {
            free_mib_per_gpu: tick.free_mib_per_gpu.clone(),
            foreign_tenant: tick.foreign_tenant,
            engine_up: tick.engine_up,
        };
        let actions = policy.decide(&sample, &scenario.config, tick.at_secs);
        assert_eq!(
            serde_json::to_value(&policy).unwrap(),
            serde_json::to_value(&result.state).unwrap()
        );
        assert_eq!(actions.len(), result.intended_actions.len());
    }
    assert_eq!(replay.executed_actions, 0);
    let kinds: Vec<_> = replay
        .decisions
        .iter()
        .flat_map(|d| &d.intended_actions)
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["relieve", "resize", "yield", "restore"]);
}

#[test]
fn empty_telemetry_and_engine_down_do_not_invent_pressure() {
    let mut scenario = fixture();
    for tick in &mut scenario.ticks {
        tick.free_mib_per_gpu.clear();
        tick.engine_up = false;
    }
    let replay = governor::replay(&scenario, "fixture".into()).unwrap();
    assert!(replay
        .decisions
        .iter()
        .all(|d| d.intended_actions.is_empty()));
}

#[test]
fn invalid_time_and_poisoned_initial_streak_are_rejected() {
    let mut scenario = fixture();
    scenario.policy.press_since = Some(i64::MIN);
    assert!(governor::replay(&scenario, "fixture".into()).is_err());
    let mut scenario = fixture();
    scenario.ticks[2].at_secs = -1;
    assert!(governor::replay(&scenario, "fixture".into()).is_err());
}
