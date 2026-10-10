//! Policy FSM ladder: Full -> Relieved -> Resized -> Yielded under sustained
//! foreign-tenant pressure; walk-back on sustained recovery; dwell guards.

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
    Config {
        confirm_secs: 10,
        resize_hold_secs: 60,
        yield_hold_secs: 180,
        recover_secs: 300,
        min_dwell_secs: 600,
        relieve_floor: 0.10,
        resize_enabled: true,
        ..Config::default()
    }
}

#[test]
fn full_to_relieved_on_pressure() {
    let mut p = Policy::default();
    let c = cfg();
    let s = sample(1000, true, true);
    p.decide(&s, &c, 1000);
    assert!(p.decide(&s, &c, 1009).is_empty());
    assert_eq!(p.decide(&s, &c, 1010), vec![Action::Relieve(0.10)]);
    assert_eq!(p.phase, Phase::Relieved);
}

#[test]
fn relieved_recovers_without_resize() {
    let mut p = Policy { phase: Phase::Relieved, ..Policy::default() };
    let c = cfg();
    let clear = sample(9000, false, true);
    p.decide(&clear, &c, 1000);
    assert!(p.decide(&clear, &c, 1299).is_empty());
    assert_eq!(p.decide(&clear, &c, 1300), vec![Action::RelieveUndo]);
    assert_eq!(p.phase, Phase::Full);
}

#[test]
fn relieved_to_resized_then_yielded() {
    let mut p = Policy { phase: Phase::Relieved, ..Policy::default() };
    let c = cfg();
    let s = sample(1000, true, true);
    // resize tier at +60s of sustained pressure
    p.decide(&s, &c, 1000);
    assert!(p.decide(&s, &c, 1059).is_empty());
    assert_eq!(p.decide(&s, &c, 1060), vec![Action::ResizeTo(212_992)]);
    assert_eq!(p.phase, Phase::Resized);
    // yield tier: +180s of pressure measured from the resize (streaks reset
    // on transition); its own hold guards it, not the resize dwell
    assert!(p.decide(&s, &c, 1100).is_empty());
    assert_eq!(p.decide(&s, &c, 1280), vec![Action::Yield]);
    assert_eq!(p.phase, Phase::Yielded);
}

#[test]
fn resized_recovers_with_full_undo() {
    let mut p = Policy { phase: Phase::Resized, ..Policy::default() };
    p.last_transition = 1000;
    let c = cfg();
    let clear = sample(9000, false, true);
    p.decide(&clear, &c, 1100); // streak starts, dwell (600) holds until 1600
    assert!(p.decide(&clear, &c, 1500).is_empty(), "streak done (300), dwell holds");
    let acts = p.decide(&clear, &c, 1600);
    assert_eq!(acts, vec![Action::ResizeUndo(262_144), Action::RelieveUndo]);
    assert_eq!(p.phase, Phase::Full);
}

#[test]
fn resize_disabled_skips_straight_to_yield() {
    let mut p = Policy { phase: Phase::Relieved, ..Policy::default() };
    let c = Config { resize_enabled: false, ..cfg() };
    let s = sample(1000, true, true);
    p.decide(&s, &c, 1000);
    assert_eq!(p.decide(&s, &c, 1060), vec![Action::Yield]);
    assert_eq!(p.phase, Phase::Yielded);
}

#[test]
fn yielded_restores_after_recovery_and_dwell() {
    let mut p = Policy { phase: Phase::Yielded, ..Policy::default() };
    p.last_transition = 1000;
    let c = cfg();
    let clear = sample(9000, false, true);
    assert!(p.decide(&clear, &c, 1200).is_empty(), "streak starts, dwell holds");
    assert!(p.decide(&clear, &c, 1500).is_empty(), "streak done (300), dwell holds");
    assert_eq!(p.decide(&clear, &c, 1600), vec![Action::Restore], "dwell passes (1600)");
    assert_eq!(p.phase, Phase::Full);
}

#[test]
fn brief_pressure_blips_never_escalate() {
    let mut p = Policy::default();
    let c = cfg();
    let dip = sample(1000, true, true);
    let ok = sample(9000, true, true); // tenant present but VRAM free
    p.decide(&dip, &c, 1000);
    p.decide(&ok, &c, 1005); // pressure streak resets
    assert!(p.decide(&dip, &c, 1012).is_empty());
    assert_eq!(p.phase, Phase::Full);
}
