//! Dashboard actions use the same profile and service commands as the CLI.

use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use crate::config::Config;

pub const ENGINE_UNIT: &str = "vitriol-server.service";
pub const AUTOSAVE_UNIT: &str = "vitriol-autosave.service";

#[derive(Debug, Clone, Copy)]
pub enum ServiceAction {
    Start,
    Restart,
    Stop,
}

impl ServiceAction {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Restart => "restart",
            Self::Stop => "stop",
        }
    }
}

pub enum Action {
    Load(String),
    Service(ServiceAction),
    Unpark(String),
}

impl Action {
    fn args(&self) -> Vec<&str> {
        match self {
            Self::Load(name) => vec!["config", "load", name],
            Self::Service(action) => vec!["service", action.verb()],
            Self::Unpark(_) => unreachable!("router actions use the router API"),
        }
    }
}

pub struct Event {
    pub ok: bool,
    pub lines: Vec<String>,
}

pub fn spawn(action: Action, cfg: &Config, tx: Sender<Event>) {
    let cli = cfg.vitriol_cli();
    thread::spawn(move || {
        if let Action::Unpark(model) = action {
            let result = crate::caduceus::unpark(&model);
            let _ = tx.send(Event {
                ok: result.is_ok(),
                lines: vec![result.err().unwrap_or_else(|| format!("Unparked {model}"))],
            });
            return;
        }
        let args = action.args();
        let mut lines = vec![format!("vitriol {}", args.join(" "))];
        let ok = match Command::new(cli).args(args).stdin(Stdio::null()).output() {
            Ok(output) => {
                for stream in [&output.stdout, &output.stderr] {
                    lines.extend(String::from_utf8_lossy(stream).lines().map(str::to_owned));
                }
                output.status.success()
            }
            Err(error) => {
                lines.push(error.to_string());
                false
            }
        };
        if ok && matches!(action, Action::Load(_)) {
            lines.push("Profile saved. Restart to apply it to a running engine.".into());
        }
        let _ = tx.send(Event { ok, lines });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_and_system_service_are_separate_commands() {
        assert_eq!(
            Action::Load("kintsugi-gpu1".into()).args(),
            ["config", "load", "kintsugi-gpu1"]
        );
        assert_eq!(
            Action::Service(ServiceAction::Restart).args(),
            ["service", "restart"]
        );
        assert_eq!(
            Action::Service(ServiceAction::Stop).args(),
            ["service", "stop"]
        );
    }
}
