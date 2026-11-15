// PROVENANCE: inspiration — none; bounded trusted-program execution for offline correctness checks.
use crate::kernel::Backend;
use std::os::unix::{
    fs::{DirBuilderExt, OpenOptionsExt},
    process::CommandExt,
};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const LIMIT: u64 = 1 << 20;

struct Temporary {
    path: PathBuf,
}

impl Temporary {
    fn new() -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vitriol-lab-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| e.to_string())?;
        Ok(Self { path })
    }

    fn file(&self, name: &str) -> Result<File, String> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.path.join(name))
            .map_err(|e| e.to_string())
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        File::open(self.path.join(name))
            .map_err(|e| e.to_string())?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT as usize {
            return Err("backend output exceeds 1 MiB".into());
        }
        Ok(bytes)
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub(crate) struct Output {
    pub status: &'static str,
    pub message: String,
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub wall_us: u64,
}

fn command(backend: &Backend, temporary: &Temporary, input: &[u8]) -> Result<Command, String> {
    use std::io::{Seek, SeekFrom, Write};
    let mut stdin = temporary.file("stdin")?;
    stdin.write_all(input).map_err(|e| e.to_string())?;
    stdin.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&backend.program);
    cmd.args(&backend.args)
        .envs(&backend.env)
        .current_dir(&temporary.path)
        .process_group(0)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(temporary.file("stdout")?))
        .stderr(Stdio::from(temporary.file("stderr")?));
    // SAFETY: the child hook only sets a POSIX resource limit; it does not allocate or acquire locks.
    unsafe {
        cmd.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: LIMIT,
                rlim_max: LIMIT,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(cmd)
}

/// Observe exit without reaping: the child's PID cannot be reused before group cleanup.
fn exited(pid: u32) -> Result<bool, String> {
    // SAFETY: waitid writes to a valid local siginfo_t; this is our unreaped child.
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let code = libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        );
        if code != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(info.si_pid() != 0)
    }
}

fn await_exit(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<(bool, std::process::ExitStatus), String> {
    let started = Instant::now();
    let observed = loop {
        match exited(child.id()) {
            Ok(true) => break Ok(false),
            Ok(false) if started.elapsed() >= timeout => break Ok(true),
            Ok(false) => std::thread::sleep(Duration::from_millis(2)),
            Err(error) => break Err(error),
        }
    };
    // SAFETY: Command created a group with the unreaped child's PID. Never target the caller's group.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    Ok((observed?, status))
}

fn classify(timeout: bool, exit: std::process::ExitStatus) -> &'static str {
    if timeout {
        return "timed_out";
    }
    if exit.code() == Some(77) {
        return "unavailable";
    }
    if exit.success() {
        return "completed";
    }
    "failed"
}

pub(crate) fn execute(backend: &Backend, input: &[u8]) -> Result<Output, String> {
    let temporary = Temporary::new()?;
    let started = Instant::now();
    let mut child = match command(backend, &temporary, input)?.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(Output {
                status: "unavailable",
                message: error.to_string(),
                stdout: vec![],
                stderr: String::new(),
                wall_us: 0,
            })
        }
    };
    let (timeout, exit) = await_exit(&mut child, Duration::from_millis(backend.timeout_ms))?;
    let status = classify(timeout, exit);
    let stderr = String::from_utf8_lossy(&temporary.read("stderr")?)
        .chars()
        .take(2048)
        .collect();
    Ok(Output {
        status,
        message: format!("exit={exit}; timeout={timeout}"),
        stdout: temporary.read("stdout")?,
        stderr,
        wall_us: started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
    })
}
