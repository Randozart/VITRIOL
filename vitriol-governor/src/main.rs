//! vitriol-governor: cooperative VRAM control plane. Phase 1 = observe +
//! YIELD/RESTORE via profile swap + systemd restart. RELIEVE (/kv-policy) and
//! RESIZE (/ctx/resize) hooks land with engine stages G2/G3.

use vitriol_governor::config::Config;
use vitriol_governor::engine;
use vitriol_governor::observe::Observer;
use vitriol_governor::policy::{Action, Policy, Sample};
use vitriol_governor::state;
use std::time::Duration;

/// CLI parsing: (config path, once, dry-run).
fn parse_cli() -> (String, bool, bool) {
    let mut cfg_path = format!(
        "{}/.vitriol/governor.toml",
        std::env::var("HOME").unwrap_or_default()
    );
    let mut once = false;
    let mut dry_run = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = args.next().unwrap_or_else(|| die("--config needs a path")),
            "--once" => once = true,
            "--dry-run" => dry_run = true,
            "--help" | "-h" => {
                println!("usage: vitriol-governor [--config PATH] [--once] [--dry-run]");
                std::process::exit(0);
            }
            other => die(&format!("unknown arg {other}")),
        }
    }
    (cfg_path, once, dry_run)
}

fn main() {
    let (cfg_path, once, dry_run) = parse_cli();
    let cfg = Config::load_or_default(&cfg_path);
    let obs = Observer::new().unwrap_or_else(|e| die(&e));
    let mut pol = state::load(&cfg.state_path);
    engine::log(&format!(
        "start phase={:?} config={cfg_path} floor={}M ceiling={}M dry_run={dry_run}",
        pol.phase, cfg.floor_free_mib, cfg.ceiling_free_mib
    ));

    loop {
        if run_tick(&mut pol, &obs, &cfg, once, dry_run) {
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// One observe -> decide -> execute -> persist cycle. Returns true when the
/// process should exit (--once mode after logging the snapshot).
fn run_tick(pol: &mut Policy, obs: &Observer, cfg: &Config, once: bool, dry_run: bool) -> bool {
    let now = chrono::Utc::now().timestamp();
    let sample = tick_sample(obs, cfg);
    for a in pol.decide(&sample, cfg, now) {
        execute(a, cfg, &sample, dry_run);
    }
    state::store(&cfg.state_path, pol);
    if once {
        engine::log(&format!(
            "once: phase={:?} free={:?} foreign={} engine_up={}",
            pol.phase, sample.free_mib_per_gpu, sample.foreign_tenant, sample.engine_up
        ));
        return true;
    }
    false
}

/// One observation: engine PID discovery + NVML sweep. The FSM acts only on
/// a healthy engine; an engine that is starting up is not `engine_up`.
fn tick_sample(obs: &Observer, cfg: &Config) -> Sample {
    let (pids, _present) = engine::engine_pids(&cfg.engine_unit);
    let healthy = engine::health_ok(cfg.engine_port);
    obs.sample(&pids, healthy, cfg.tenant_min_mib)
}

fn execute(a: Action, cfg: &Config, s: &Sample, dry_run: bool) {
    match a {
        Action::Note(m) => engine::log(&m),
        Action::Yield => execute_yield(cfg, s, dry_run),
        Action::Restore => execute_restore(cfg, s, dry_run),
    }
}

/// YIELD: swap to the smaller profile + restart the unit (warm KV restore),
/// then demote the engine from last-OOM-victim so tenants can win VRAM.
fn execute_yield(cfg: &Config, s: &Sample, dry_run: bool) {
    engine::log(&format!(
        "PRESSURE confirmed (min free {} MiB < floor {} MiB, foreign tenant resident) -> YIELD: load {} + restart {}",
        s.min_free(), cfg.floor_free_mib, cfg.yield_profile, cfg.engine_unit
    ));
    if dry_run {
        engine::log("dry-run: yield not executed");
        return;
    }
    if let Err(e) = engine::config_load(&cfg.project_dir, &cfg.yield_profile) {
        engine::log(&format!("ERROR config load: {e} - ABORT yield"));
        return;
    }
    if let Err(e) = engine::unit_restart(&cfg.engine_unit) {
        engine::log(&format!("ERROR restart: {e} - ABORT"));
        return;
    }
    if !engine::wait_health(cfg.engine_port, cfg.health_timeout_secs) {
        engine::log("ERROR engine not healthy after yield restart");
        return;
    }
    let p = engine::props(cfg.engine_port);
    engine::log(&format!(
        "yield complete, engine healthy: {}",
        p.as_ref()
            .map(|v| format!(
                "model={} n_ctx={}",
                v["model_path"].as_str().unwrap_or("?"),
                v["default_generation_settings"]["n_ctx"].as_i64().unwrap_or(-1)
            ))
            .unwrap_or_else(|| "props unavailable".into())
    ));
    engine::apply_yield_oom(cfg);
}

/// RESTORE: walk back to the full profile; the unit file re-applies
/// OOMScoreAdjust=-500 so the engine regains last-OOM-victim status.
fn execute_restore(cfg: &Config, s: &Sample, dry_run: bool) {
    engine::log(&format!(
        "RECOVERY confirmed (min free {} MiB >= ceiling {} MiB, no foreign tenant) -> RESTORE: load {} + restart {}",
        s.min_free(), cfg.ceiling_free_mib, cfg.full_profile, cfg.engine_unit
    ));
    if dry_run {
        engine::log("dry-run: restore not executed");
        return;
    }
    if let Err(e) = engine::config_load(&cfg.project_dir, &cfg.full_profile) {
        engine::log(&format!("ERROR config load: {e} - ABORT restore"));
        return;
    }
    if let Err(e) = engine::unit_restart(&cfg.engine_unit) {
        engine::log(&format!("ERROR restart: {e} - ABORT"));
        return;
    }
    if engine::wait_health(cfg.engine_port, cfg.health_timeout_secs) {
        engine::log("restore complete (unit re-applies OOMScoreAdjust=-500)");
    } else {
        engine::log("ERROR engine not healthy after restore restart");
    }
}

fn die(msg: &str) -> ! {
    engine::log(&format!("FATAL {msg}"));
    std::process::exit(1);
}
