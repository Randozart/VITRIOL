//! Policy FSM transitions: yield on sustained foreign-tenant pressure,
//! restore on sustained recovery, dwell + streak-reset guards.

use vitriol_governor::config::Config;
use vitriol_governor::policy::{Action, Phase, Policy, Sample};

fn sample(min_free: u64, foreign: bool, up: bool) -> Sample {
    Sample {
        free_mib_per_gpu: vec![min_free, min_free + 512],
        foreign_tenant: foreign,
        engine_up: up,
    }
}

fn cfg() -> Config {
    Config { confirm_secs: 10, recover_secs: 300, min_dwell_secs: 600, ..Config::default() }
}

#[test]
fn yield_fires_after_confirm_window() {
    let mut p = Policy::default();
    let c = cfg();
    let s = sample(1000, true, true); // 1 GiB free < 1.5 GiB floor
    assert!(p.decide(&s, &c, 1000).is_empty(), "no action before confirm");
    assert!(p.decide(&s, &c, 1009).is_empty(), "no action inside confirm");
    let acts = p.decide(&s, &c, 1010);
    assert_eq!(acts, vec![Action::Yield], "yield at confirm boundary");
    assert_eq!(p.phase, Phase::Yielded);
}

#[test]
fn brief_dip_does_not_yield() {
    let mut p = Policy::default();
    let c = cfg();
    let dip = sample(1000, true, true);
    p.decide(&dip, &c, 1000);
    p.decide(&sample(8000, true, true), &c, 1005); // condition broke: streak resets
    assert!(p.decide(&dip, &c, 1012).is_empty(), "streak was reset mid-window");
}

#[test]
fn no_yield_without_foreign_tenant() {
    let mut p = Policy::default();
    let c = cfg();
    let s = sample(500, false, true); // engine hogs itself: normal operation
    p.decide(&s, &c, 1000);
    assert!(p.decide(&s, &c, 2000).is_empty());
    assert_eq!(p.phase, Phase::Full);
}

#[test]
fn no_yield_when_engine_down() {
    let mut p = Policy::default();
    let c = cfg();
    let s = sample(500, true, false);
    p.decide(&s, &c, 1000);
    assert!(p.decide(&s, &c, 2000).is_empty());
}

#[test]
fn dwell_blocks_restore_then_recover_window_applies() {
    let mut p = Policy::default();
    let c = cfg();
    let dip = sample(1000, true, true);
    p.decide(&dip, &c, 1000);
    assert_eq!(p.decide(&dip, &c, 1010), vec![Action::Yield]); // last_transition = 1010

    let clear = sample(9000, false, true);
    // streak and dwell accumulate in parallel; action fires when both pass
    assert!(p.decide(&clear, &c, 1100).is_empty(), "streak starts, dwell holds");
    assert!(p.decide(&clear, &c, 1400).is_empty(), "streak done (300), dwell holds");
    assert_eq!(p.decide(&clear, &c, 1700), vec![Action::Restore], "dwell passes (1610)");
    assert_eq!(p.phase, Phase::Full);
}

#[test]
fn restore_requires_sustained_clear() {
    let mut p = Policy { phase: Phase::Yielded, ..Policy::default() };
    p.last_transition = 1000;
    let c = cfg();
    let clear = sample(9000, false, true);
    let busy = sample(9000, true, true); // tenant returns mid-recovery
    p.decide(&clear, &c, 2000); // streak starts
    p.decide(&busy, &c, 2200); // streak resets
    assert!(p.decide(&clear, &c, 2400).is_empty(), "new streak starts");
    assert!(p.decide(&clear, &c, 2699).is_empty());
    assert_eq!(p.decide(&clear, &c, 2700), vec![Action::Restore], "2400 + 300 recover");
}
