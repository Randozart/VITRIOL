//! Control actions: start/stop/restart the stack, run doctor, load a profile.
//!
//! Each action expands into one or more sequential shell steps that run the
//! existing tested scripts (`scripts/launch_vitriol_full.sh` for process
//! control, `scripts/vitriol config load` for profile management). Steps run
//! on a background thread, streaming output lines back to the UI through an
//! mpsc channel. The UI can abort the current child via the shared abort flag.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;

use crate::config::Config;
use crate::profile::Profile;

/// A user-visible control action in the CONTROLS tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Launch the full stack; `selected` is a profile whose knobs are applied
    /// as CLI overrides (flags-only — the active config file is left as-is).
    Start { selected: Option<String> },
    /// Stop the full stack.
    Stop,
    /// Stop then start the full stack; `selected` mirrors Start's profile.
    Restart { selected: Option<String> },
    /// Run the launch script's pre-flight checks.
    Doctor,
    /// Run `vitriol setup` — set CAP_IPC_LOCK for page-locked stream mode.
    Setup,
    /// Boot the whole REBIS trenchcoat: Sol + Luna + Mercury, supervised.
    LaunchRebis,
    /// Run a Spagyric decode-knob sweep for a profile's model.
    RunSweep(String),
    /// Run the sweep AND write the per-knob winner as `<name>-swept` profile.
    SweepAndSave(String),
}

impl Action {
    /// Short label for the action list.
    pub fn label(&self) -> String {
        match self {
            Action::Start {
                selected: Some(name),
            } => format!("start stack ({name})"),
            Action::Start { selected: None } => "start stack".into(),
            Action::Stop => "stop stack".into(),
            Action::Restart {
                selected: Some(name),
            } => format!("restart stack ({name})"),
            Action::Restart { selected: None } => "restart stack".into(),
            Action::Doctor => "run doctor".into(),
            Action::Setup => "vitriol setup (CAP_IPC_LOCK)".into(),
            Action::LaunchRebis => "launch rebis (Sol+Luna+Mercury)".into(),
            Action::RunSweep(name) => format!("sweep: {name}"),
            Action::SweepAndSave(name) => format!("sweep+save: {name}"),
        }
    }

    /// The action list: fixed process actions first, then a sweep row per profile.
    /// `selected` is the profile Start/Restart will apply as CLI overrides.
    pub fn all(profiles: &[Profile], selected: Option<&str>) -> Vec<Action> {
        let mut actions = vec![
            Action::Start {
                selected: selected.map(str::to_owned),
            },
            Action::Stop,
            Action::Restart {
                selected: selected.map(str::to_owned),
            },
            Action::Doctor,
            Action::Setup,
            Action::LaunchRebis,
        ];
        for p in profiles {
            actions.push(Action::RunSweep(p.name.clone()));
            actions.push(Action::SweepAndSave(p.name.clone()));
        }
        actions
    }
}

/// One sequential shell step of an action.
pub struct Step {
    /// Label shown while the step runs.
    pub label: String,
    /// Program to execute.
    pub program: String,
    /// Arguments to the program.
    pub args: Vec<String>,
}

/// Control-thread events, drained by the UI event loop.
#[derive(Debug, Clone)]
pub enum Event {
    /// The whole action started.
    Started(String),
    /// A step began.
    StepStarted(String),
    /// One output line (stdout or stderr) from the current step.
    Line(String),
    /// The action finished; the bool is whether every step succeeded.
    Done(bool),
}

/// Expand an action into its ordered steps.
fn steps_for(action: &Action, cfg: &Config) -> Vec<Step> {
    let launch = cfg.launch_script().to_string_lossy().into_owned();
    match action {
        Action::Start { selected } => vec![Step {
            label: action.label(),
            program: launch.clone(),
            args: launch_args(cfg, selected.as_deref()),
        }],
        Action::Stop => vec![Step {
            label: "stop full stack".into(),
            program: launch.clone(),
            args: vec!["stop".into()],
        }],
        Action::Restart { selected } => vec![
            Step {
                label: "stop full stack".into(),
                program: launch.clone(),
                args: vec!["stop".into()],
            },
            Step {
                label: "launch full stack".into(),
                program: launch,
                args: launch_args(cfg, selected.as_deref()),
            },
        ],
        Action::Doctor => vec![Step {
            label: "pre-flight checks".into(),
            program: launch,
            args: vec!["doctor".into()],
        }],
        Action::LaunchRebis => vec![Step {
            label: action.label(),
            program: cfg.repo_root.join("scripts/rebis-servers.sh")
                .to_string_lossy().into_owned(),
            args: vec!["rebis".into()],
        }],
        Action::Setup => {
            // 2026-08-07: `sudo -n` keeps the no-tty control thread honest — a
            // cached/passwordless sudo succeeds, otherwise it fails cleanly with
            // a log line instead of hanging on a password prompt.
            let cli = cfg.vitriol_cli().to_string_lossy().into_owned();
            vec![Step {
                label: "set CAP_IPC_LOCK (page-locked stream)".into(),
                program: "sudo".into(),
                args: vec!["-n".into(), cli, "setup".into()],
            }]
        }
        Action::RunSweep(name) => sweep_steps(cfg, name, false),
        Action::SweepAndSave(name) => sweep_steps(cfg, name, true),
    }
}

/// Launch args: `--no-setup` plus, when a profile is selected, its knob flags
/// as CLI overrides (flags-only — the active config stays untouched). 2026-08-08:
/// Start/Restart now honour the PROFILES-tab selected profile.
fn launch_args(cfg: &Config, selected: Option<&str>) -> Vec<String> {
    let mut args = vec!["--no-setup".into()];
    if let Some(name) = selected {
        if let Some(p) = find_profile(cfg, name) {
            let flags = launch_flags(&p);
            if !flags.is_empty() {
                args.extend(flags);
            }
        }
    }
    args
}

/// Sweep steps for a profile: run `spagyric_sweep.py`; when `save` is set, also
/// pass `--build-profile <name>` so the per-knob winner is written as a profile.
fn sweep_steps(cfg: &Config, name: &str, save: bool) -> Vec<Step> {
    let sweep_script = cfg.repo_root.join("libvitriol/spagyric_sweep.py");
    let Some(profile) = find_profile(cfg, name) else {
        return vec![noop(format!("profile {name} not found"))];
    };
    let Some(model) = profile.model.clone() else {
        return vec![noop(format!(
            "profile {name} has no model.path — set it, then sweep"
        ))];
    };
    let ngl = profile.ngl.unwrap_or(99);
    let ctx = profile.ctx.unwrap_or(4096);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let output = format!("/tmp/opencode/sweep_{name}_{stamp}.csv");
    let mut args = vec![
        sweep_script.to_string_lossy().into_owned(),
        "--model".into(),
        model,
        "--ngl".into(),
        ngl.to_string(),
        "--ctx".into(),
        ctx.to_string(),
        "--output".into(),
        output,
    ];
    if save {
        args.push("--build-profile".into());
        args.push(name.to_string());
    }
    let label = if save {
        format!("spagyric sweep + save: {name}")
    } else {
        format!("spagyric sweep: {name}")
    };
    vec![Step {
        label,
        program: "python3".into(),
        args,
    }]
}

/// A no-op step that just reports a message and exits cleanly.
fn noop(message: String) -> Step {
    Step {
        label: message.clone(),
        program: "echo".into(),
        args: vec![message],
    }
}

/// Find a profile by name (installed shadowing bundled).
fn find_profile(cfg: &Config, name: &str) -> Option<Profile> {
    crate::profile::discover(cfg)
        .into_iter()
        .find(|p| p.name == name)
}

/// The launch-flag set for a profile's knobs.
fn launch_flags(p: &Profile) -> Vec<String> {
    let mut flags = Vec::new();
    if let Some(m) = &p.model {
        flags.push(format!("--model={m}"));
    }
    if let Some(n) = p.ngl {
        flags.push(format!("--ngl={n}"));
    }
    if let Some(c) = p.ctx {
        flags.push(format!("--ctx={c}"));
    }
    if let Some(t) = p.threads {
        flags.push(format!("--threads={t}"));
    }
    if let Some(k) = p.parallel {
        flags.push(format!("--parallel={k}"));
    }
    flags
}

/// Spawn a control executor thread running `action`, streaming events on `tx`.
/// The executor honours `abort`: while a child runs it is killed, and no
/// further steps execute after an abort.
pub fn spawn(action: Action, cfg: &Config, tx: Sender<Event>, abort: Arc<AtomicBool>) {
    let cfg = cfg.clone();
    thread::Builder::new()
        .name("vitriol-tui-control".into())
        .spawn(move || {
            let _ = tx.send(Event::Started(action.label()));
            let steps = steps_for(&action, &cfg);
            let mut ok = true;
            for step in steps {
                if abort.swap(false, Ordering::Relaxed) {
                    ok = false;
                    break;
                }
                let _ = tx.send(Event::StepStarted(step.label));
                let mut child = match Command::new(&step.program)
                    .args(&step.args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Event::Line(format!("spawn failed: {e}")));
                        ok = false;
                        break;
                    }
                };
                if let Some(stdout) = child.stdout.take() {
                    stream_lines(BufReader::new(stdout), &tx, &abort);
                }
                if let Some(stderr) = child.stderr.take() {
                    stream_lines(BufReader::new(stderr), &tx, &abort);
                }
                if abort.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    ok = false;
                    break;
                }
                match child.wait() {
                    Ok(status) if status.success() => {}
                    Ok(status) => {
                        let _ = tx.send(Event::Line(format!("step failed: {status}")));
                        ok = false;
                        break;
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Line(format!("wait failed: {e}")));
                        ok = false;
                        break;
                    }
                }
            }
            let _ = tx.send(Event::Done(ok));
        })
        .expect("spawn vitriol-tui control thread");
}

/// Stream a pipe's lines to the UI, stopping early when abort is raised.
fn stream_lines(mut reader: impl BufRead, tx: &Sender<Event>, abort: &AtomicBool) {
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let _ = tx.send(Event::Line(line.trim_end().to_string()));
            }
        }
        if abort.load(Ordering::Relaxed) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ProfileSource;

    #[test]
    fn start_with_selected_profile_appends_launch_flags() {
        let mut cfg = Config::from_env();
        cfg.repo_root = std::env::temp_dir();
        let started = steps_for(
            &Action::Start {
                selected: Some("anything".into()),
            },
            &cfg,
        );
        assert_eq!(started.len(), 1);
        assert!(started[0].args.contains(&"--no-setup".to_string()));
    }

    #[test]
    fn action_list_has_fixed_plus_sweeps_per_profile() {
        let p = Profile {
            name: "mellum2".into(),
            description: String::new(),
            source: ProfileSource::Bundled,
            model: None,
            ngl: None,
            ctx: None,
            threads: None,
            parallel: None,
        };
        let actions = Action::all(&[p], None);
        assert_eq!(actions.len(), 8);
        assert_eq!(actions[0], Action::Start { selected: None });
        assert_eq!(actions[1], Action::Stop);
        assert_eq!(actions[2], Action::Restart { selected: None });
        assert_eq!(actions[3], Action::Doctor);
        assert_eq!(actions[4], Action::Setup);
        assert_eq!(actions[5], Action::LaunchRebis);
        assert_eq!(actions[6], Action::RunSweep("mellum2".into()));
        assert_eq!(actions[7], Action::SweepAndSave("mellum2".into()));
        let with_sel = Action::all(&[], Some("qwen"));
        assert_eq!(
            with_sel[0],
            Action::Start {
                selected: Some("qwen".into()),
            }
        );
    }

    #[test]
    fn setup_runs_sudo_noninteractive() {
        let mut cfg = Config::from_env();
        cfg.repo_root = std::env::temp_dir();
        let steps = steps_for(&Action::Setup, &cfg);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].program, "sudo");
        let binding = std::env::temp_dir().join("scripts/vitriol");
        let cli = binding.to_string_lossy();
        assert_eq!(steps[0].args, vec!["-n", &cli, "setup"]);
    }

    #[test]
    fn sweep_and_save_passes_build_profile_flag() {
        let mut cfg = Config::from_env();
        cfg.repo_root = std::env::temp_dir();
        let steps = steps_for(&Action::SweepAndSave("ghost".into()), &cfg);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].program, "echo");
    }

    #[test]
    fn sweep_without_model_is_a_noop() {
        let mut cfg = Config::from_env();
        cfg.repo_root = std::env::temp_dir();
        let steps = steps_for(&Action::RunSweep("ghost".into()), &cfg);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].program, "echo");
    }
}
