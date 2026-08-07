//! Officina — the model-surgery workshop REPL (Alka / SPQL).
//!
//! A transactional command environment: the model is a queryable database and
//! every edit is a composable operation. Mutating commands run as PROBEs by
//! default (impact table, no mutation); a `COMMIT >` prefix applies them.
//! Committed operations land in the transformation journal (undoable) and, when
//! recording, into a grimoire recipe file.
//!
//! Only operations with a real backend are present: DESCRIBE (GGUF metadata
//! census), TEST (live gen server), MAP (system memory), COMPILE (`.spagyr`
//! bundle), RECORD/STOP/PLAY (grimoires), UNDO, CLEAR, HELP. Weight surgery
//! (DISSOLVE/COAGULATE) reports its backend is not yet built — never a fake
//! result.

pub mod config;
pub mod grammar;
pub mod grimoire;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::model::Snapshot;
use crate::officina::config::OfficinaConfig;
use crate::officina::grammar::{Command, Keyword, ParseError};

/// One committed transformation journal entry.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    /// Human-readable summary of the op.
    pub text: String,
    /// Estimated logit drift contribution (0 for read-only ops).
    pub drift_delta: f64,
}

/// Context an op needs from the rest of the app.
pub struct OpCtx<'a> {
    /// App config (ports, home, repo paths).
    pub cfg: &'a Config,
    /// Latest live snapshot.
    pub snap: &'a Snapshot,
    /// Resolved active model file path (None when unconfigured).
    pub model_path: Option<PathBuf>,
    /// Active profile name (None when none selected).
    pub profile: Option<String>,
}

/// Officina REPL state.
pub struct Officina {
    /// Home dir (config, grimoires, spagyr outputs).
    home: PathBuf,
    /// Telemetry + style configuration.
    pub config: OfficinaConfig,
    /// Current command line being typed.
    pub input: String,
    /// Rendered output scrollback.
    pub output: VecDeque<String>,
    /// Command history for up-arrow recall.
    history: VecDeque<String>,
    /// Cursor into history while recalling.
    history_pos: Option<usize>,
    /// Transformation journal (committed ops).
    pub journal: Vec<JournalEntry>,
    /// Cumulative estimated logit drift.
    pub drift: f64,
    /// Whether the active model file has been mutated (P3).
    pub model_dirty: bool,
    /// Active grimoire being recorded (None when not recording).
    pub recording: Option<String>,
    /// Captured committed-op lines for the active recording.
    recipe: Vec<String>,
}

impl Officina {
    /// Build a fresh Officina session, loading config from `~/.vitriol`.
    pub fn new(home: &Path) -> Self {
        let config = OfficinaConfig::load(&home.join(".vitriol/officina.toml"));
        Self {
            home: home.to_path_buf(),
            config,
            input: String::new(),
            output: VecDeque::with_capacity(500),
            history: VecDeque::with_capacity(200),
            history_pos: None,
            journal: Vec::new(),
            drift: 0.0,
            model_dirty: false,
            recording: None,
            recipe: Vec::new(),
        }
    }

    /// Append a character to the command line.
    pub fn type_char(&mut self, c: char) {
        self.input.push(c);
        self.history_pos = None;
    }

    /// Remove the last character from the command line.
    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Walk command history (delta +1 down, -1 up), restoring into `input`.
    pub fn history_nav(&mut self, delta: isize) {
        if self.history.is_empty() {
            return;
        }
        let len = self.history.len() as isize;
        let cur = match self.history_pos {
            Some(p) => p as isize,
            None => {
                self.history.push_back(self.input.clone());
                len - 1
            }
        };
        let next = (cur + delta).clamp(0, len - 1);
        self.history_pos = Some(next as usize);
        self.input = self.history[next as usize].clone();
    }

    /// The rendered prompt header telemetry string (top line).
    pub fn prompt_header(&self, ctx: &OpCtx) -> String {
        let mut blocks: Vec<String> = Vec::new();
        let c = &self.config;
        if c.show_model {
            let name = ctx
                .profile
                .clone()
                .or_else(|| {
                    ctx.model_path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|f| f.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| "no-model".into());
            let state = if self.model_dirty { "dirty" } else { "clean" };
            blocks.push(format!("model: {name}: {state}"));
        }
        if c.show_context {
            let n = ctx.snap.gen.n_ctx.unwrap_or(0);
            blocks.push(format!("context: {n}"));
        }
        if c.show_drift {
            blocks.push(format!("drift: {:.4}", self.drift));
        }
        if c.show_vram {
            let used = gpu_mib(ctx.snap, |g| g.vram_used_mib) as f64 / 1024.0;
            let total = gpu_mib(ctx.snap, |g| g.vram_total_mib) as f64 / 1024.0;
            blocks.push(format!("vram: {used:.1}G/{total:.1}G"));
        }
        if c.show_experts {
            blocks.push("experts: ?/64".into());
        }
        if blocks.is_empty() {
            "ALKA".into()
        } else {
            blocks.join("]-[")
        }
    }

    /// Run a raw input line: parse + execute, returning new output lines.
    pub fn run(&mut self, line: &str, ctx: &OpCtx) -> Vec<String> {
        self.history_pos = None;
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("//") {
            self.history.push_back(trimmed.to_string());
            if self.history.len() > 200 {
                self.history.pop_front();
            }
        }
        match grammar::parse(line) {
            Err(ParseError::Empty) => Vec::new(),
            Err(e) => vec![format!("[ERR] {e}")],
            Ok(cmd) => self.execute(&cmd, ctx),
        }
    }

    /// Dispatch one parsed command to its backend.
    fn execute(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        match cmd.keyword {
            Keyword::Help => help_lines(),
            Keyword::Clear => {
                self.output.clear();
                Vec::new()
            }
            Keyword::Undo => self.undo(),
            Keyword::Record => self.record(cmd),
            Keyword::Stop => self.stop_recording(),
            Keyword::Play => self.play(cmd, ctx),
            Keyword::Compile => self.compile(cmd, ctx),
            Keyword::Describe => self.describe(cmd, ctx),
            Keyword::Census => self.census(cmd, ctx),
            Keyword::Map => self.map(ctx),
            Keyword::Test => self.test(cmd, ctx),
            Keyword::Dissolve | Keyword::Coagulate => vec![format!(
                "[ERR] {}: weight surgery needs the P3 offline-rewrite backend — not built yet",
                cmd.keyword.as_str()
            )],
        }
    }

    /// Record a committed mutating op into the recipe/journal.
    fn record_op(&mut self, text: &str, raw: &str, drift_delta: f64) {
        self.journal.push(JournalEntry {
            text: text.to_string(),
            drift_delta,
        });
        self.drift = (self.drift + drift_delta).max(0.0);
        if let Some(_name) = &self.recording {
            if !raw.trim().starts_with("COMMIT") {
                self.recipe.push(format!("COMMIT > {raw}"));
            } else {
                self.recipe.push(raw.to_string());
            }
        }
    }

    /// UNDO: revert the last committed transformation.
    fn undo(&mut self) -> Vec<String> {
        let Some(last) = self.journal.pop() else {
            return vec!["[UNDO] nothing to undo".into()];
        };
        self.drift = (self.drift - last.drift_delta).max(0.0);
        vec![format!("[UNDO] reverted: {}", last.text)]
    }

    /// RECORD: begin capturing committed ops into a grimoire.
    fn record(&mut self, cmd: &Command) -> Vec<String> {
        if self.recording.is_some() {
            return vec!["[ERR] already recording — STOP first".into()];
        }
        if cmd.target.is_empty() {
            return vec!["[ERR] RECORD > requires a grimoire name".into()];
        }
        if !cmd
            .target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return vec![format!("[ERR] invalid grimoire name '{}'", cmd.target)];
        }
        self.recording = Some(cmd.target.clone());
        self.recipe.clear();
        vec![format!(
            "[RECORDING] writing committed ops to {}",
            cmd.target
        )]
    }

    /// STOP: write the recorded recipe to disk.
    fn stop_recording(&mut self) -> Vec<String> {
        let Some(name) = self.recording.take() else {
            return vec!["[ERR] no active recording".into()];
        };
        let lines = std::mem::take(&mut self.recipe);
        match grimoire::write(&self.home, &name, &lines) {
            Ok(path) => vec![format!(
                "[SAVED] {} ({} lines)",
                path.display(),
                lines.len()
            )],
            Err(e) => vec![format!("[ERR] {e}")],
        }
    }

    /// PLAY: read a grimoire; probe lists it, COMMIT runs it.
    fn play(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if cmd.target.is_empty() {
            let avail = grimoire::list(&self.home);
            let hint = if avail.is_empty() {
                "none recorded".into()
            } else {
                avail.join(", ")
            };
            return vec![format!(
                "[ERR] PLAY > requires a grimoire name or path — available: {hint}"
            )];
        }
        let path = self.resolve_grimoire(&cmd.target);
        let lines = match grimoire::read(&path) {
            Ok(l) => l,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        let mode = if cmd.commit { "COMMIT" } else { "PROBE" };
        let mut out = vec![format!(
            "[{mode}] PLAY {} ({} instructions)",
            path.display(),
            lines.len()
        )];
        for line in &lines {
            if cmd.commit {
                let applied = self.run_committed(line, ctx);
                out.extend(applied);
            } else {
                out.push(format!("  {line}"));
            }
        }
        out
    }

    /// Run one grimoire line in commit mode: prefix mutating ops with COMMIT so
    /// they apply rather than probe.
    fn run_committed(&mut self, line: &str, ctx: &OpCtx) -> Vec<String> {
        match grammar::parse(line) {
            Ok(cmd) if !cmd.keyword.is_read_only() && !cmd.commit => {
                self.run(&format!("COMMIT > {line}"), ctx)
            }
            _ => self.run(line, ctx),
        }
    }

    /// Resolve a grimoire target to an absolute path.
    fn resolve_grimoire(&self, target: &str) -> PathBuf {
        let p = Path::new(target);
        if p.is_absolute() || target.contains('/') {
            p.to_path_buf()
        } else {
            grimoire::grimoires_dir(&self.home).join(format!("{target}.grimoire"))
        }
    }

    /// COMPILE: package grimoire ref + model fingerprint + profile into a
    /// `.spagyr` bundle (real artifact; AOT backend deferred).
    fn compile(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if cmd.target.is_empty() {
            return vec!["[ERR] COMPILE > requires a bundle name".into()];
        }
        let fingerprint = ctx
            .model_path
            .as_ref()
            .map(|p| {
                let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                format!(
                    "{}@{size}B",
                    p.file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "no-model".into());
        let bundle = serde_json::json!({
            "name": cmd.target,
            "kind": "spagyr-bundle",
            "created": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            "grimoire": self.recording.clone(),
            "model": fingerprint,
            "profile": ctx.profile,
        });
        let dir = self.home.join(".vitriol/spagyr");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return vec![format!("[ERR] mkdir: {e}")];
        }
        let path = dir.join(format!("{}.spagyr", cmd.target));
        let text = serde_json::to_string_pretty(&bundle).unwrap_or_default();
        match std::fs::write(&path, text) {
            Ok(()) => {
                self.record_op(&format!("compiled {}", cmd.target), &cmd.raw, 0.0);
                vec![format!(
                    "[COMMITTED] package bundle written: {}",
                    path.display()
                )]
            }
            Err(e) => vec![format!("[ERR] write: {e}")],
        }
    }

    /// DESCRIBE: GGUF metadata census of the active model. Target `model` (or
    /// empty) shows the aggregate; `layer.N` / `layer.N.mlp` shows the catalog
    /// rows for that layer's tensors.
    fn describe(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        let Some(path) = &ctx.model_path else {
            return vec!["[ERR] no model path configured (set [model] path)".into()];
        };
        let info = match vitriol_calibrate::gguf::read_gguf(path) {
            Ok(info) => info,
            Err(e) => return vec![format!("[ERR] read gguf: {e}")],
        };
        let target = cmd.target.trim().to_lowercase();
        if target.is_empty() || target == "model" {
            return describe_aggregate(path, &info);
        }
        describe_layer(&info, &target)
    }

    /// CENSUS: W0 value census of a layer's tensors (dead-lane %, entropy).
    fn census(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        let Some(path) = &ctx.model_path else {
            return vec!["[ERR] no model path configured (set [model] path)".into()];
        };
        let info = match vitriol_calibrate::gguf::read_gguf(path) {
            Ok(info) => info,
            Err(e) => return vec![format!("[ERR] read gguf: {e}")],
        };
        let target = cmd.target.trim().to_lowercase();
        let entries: Vec<&vitriol_calibrate::gguf::TensorEntry> =
            if target.is_empty() || target == "model" {
                info.tensors.iter().take(CENSUS_TENSOR_CAP).collect()
            } else {
                match matching_tensors(&info, &target) {
                    Some(list) => list,
                    None => {
                        return vec![format!(
                        "[ERR] target '{target}' — expected 'layer.N' or 'layer.N.mlp|norm|attn'"
                    )]
                    }
                }
            };
        if entries.is_empty() {
            return vec![format!("[PROBE] CENSUS {target} — no matching tensors")];
        }
        let mut out = vec![format!(
            "[PROBE] CENSUS {target} — {} tensors",
            entries.len()
        )];
        let mut agg_dead: f64 = 0.0;
        let mut agg_elems: u64 = 0;
        for t in entries {
            match vitriol_calibrate::census::census_tensor(path, t) {
                Ok(c) if c.unsupported => {
                    out.push(format!(
                        "  ├── {:<40} {:>10} unsupported for value census",
                        t.name,
                        vitriol_calibrate::gguf::type_name(t.ggml_type)
                    ));
                }
                Ok(c) => {
                    agg_dead += c.zero_fraction * c.sampled as f64;
                    agg_elems += c.sampled;
                    out.push(format!(
                        "  ├── {:<40} {:>10} dead {:>5.1}%  ent {:.2} bits  |x|̄ {:.3e}",
                        t.name,
                        vitriol_calibrate::gguf::type_name(t.ggml_type),
                        c.zero_fraction * 100.0,
                        c.entropy_bits,
                        c.abs_mean
                    ));
                }
                Err(e) => {
                    out.push(format!("  ├── {} read error: {e}", t.name));
                }
            }
        }
        if agg_elems > 0 {
            out.push(format!(
                "  └── aggregate dead lanes: {:.1}% ({} sampled)",
                agg_dead / agg_elems as f64 * 100.0,
                agg_elems
            ));
        }
        out
    }

    /// MAP: real system memory layout.
    fn map(&mut self, ctx: &OpCtx) -> Vec<String> {
        let used_mib = gpu_mib(ctx.snap, |g| g.vram_used_mib);
        let total_mib = gpu_mib(ctx.snap, |g| g.vram_total_mib);
        let host = read_meminfo();
        let mut out = vec!["[PROBE] MAP — system memory".into()];
        out.push(format!(
            "  ├── VRAM: {:.1}/{:.1} GiB ({:.0}%)",
            used_mib as f64 / 1024.0,
            total_mib as f64 / 1024.0,
            if total_mib > 0 {
                used_mib as f64 / total_mib as f64 * 100.0
            } else {
                0.0
            }
        ));
        out.push(format!(
            "  ├── Host RAM: {:.1}/{:.1} GiB free",
            host.available_mib as f64 / 1024.0,
            host.total_mib as f64 / 1024.0
        ));
        let hermetis = &ctx.snap.hermetis;
        out.push(format!(
            "  ├── Hermetis: {} episodes, {} nodes, {} sessions",
            hermetis.episodes.unwrap_or(0),
            hermetis.nodes.unwrap_or(0),
            hermetis.sessions.unwrap_or(0)
        ));
        out.push(format!(
            "  └── context: {} · decode: {:.1} t/s",
            ctx.snap.gen.n_ctx.unwrap_or(0),
            ctx.snap.gen.decode_t_s
        ));
        out
    }

    /// TEST: run a prompt through the live gen server and syntax-check.
    fn test(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if cmd.target.is_empty() {
            return vec!["[ERR] TEST > \"prompt\" requires quoted text".into()];
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(120))
            .build();
        let url = format!("{}/v1/completions", ctx.cfg.gen_base());
        let body = serde_json::json!({
            "prompt": cmd.target,
            "n_predict": 128,
            "temperature": 0.0,
        });
        let Ok(resp) = agent.post(&url).send_json(body) else {
            return vec![format!(
                "[ERR] gen server unreachable at {} — start the stack first",
                ctx.cfg.gen_base()
            )];
        };
        let Ok(payload) = resp.into_json::<serde_json::Value>() else {
            return vec!["[ERR] bad gen response".into()];
        };
        let text = payload
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        let tps = payload
            .get("timings")
            .and_then(|t| t.get("predicted_per_second"))
            .and_then(|t| t.as_f64())
            .unwrap_or(0.0);
        let mut out = vec![format!("[TESTING] decode {tps:.1} t/s")];
        let trimmed = text.trim();
        if trimmed.is_empty() {
            out.push("  └── empty output".into());
        } else {
            let snippet: String = trimmed.chars().take(200).collect();
            for line in snippet.lines().take(6) {
                out.push(format!("  │ {line}"));
            }
            out.push(format!("  └── syntax: {}", syntax_check(trimmed)));
        }
        out
    }
}

/// A quick structural syntax check on generated code: brace/paren balance.
fn syntax_check(code: &str) -> &'static str {
    let mut braces = 0i32;
    let mut parens = 0i32;
    let mut brackets = 0i32;
    for c in code.chars() {
        match c {
            '{' => braces += 1,
            '}' => braces -= 1,
            '(' => parens += 1,
            ')' => parens -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            _ => {}
        }
    }
    if braces == 0 && parens == 0 && brackets == 0 {
        "BALANCED"
    } else {
        "UNBALANCED"
    }
}

/// Host memory totals from `/proc/meminfo` (0 on failure).
fn read_meminfo() -> MemInfo {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return MemInfo::default();
    };
    let mut m = MemInfo::default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let key = it.next().unwrap_or("");
        let val = it.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        match key {
            "MemTotal:" => m.total_mib = val / 1024,
            "MemAvailable:" => m.available_mib = val / 1024,
            _ => {}
        }
    }
    m
}

/// Host RAM snapshot.
#[derive(Default)]
struct MemInfo {
    total_mib: u64,
    available_mib: u64,
}

/// A GPU field value, or 0 when the GPU snapshot is absent.
fn gpu_mib(snap: &Snapshot, pick: impl Fn(&crate::model::GpuSnapshot) -> u64) -> u64 {
    snap.gpu.as_ref().map(pick).unwrap_or(0)
}

/// Render the aggregate model census.
fn describe_aggregate(path: &Path, info: &vitriol_calibrate::gguf::ModelInfo) -> Vec<String> {
    vec![
        format!("[PROBE] DESCRIBE {}", path.display()),
        format!("  ├── arch: {}", info.architecture),
        format!("  ├── layers: {}", info.block_count),
        format!(
            "  ├── experts: {}/{}",
            info.expert_used_count, info.expert_count
        ),
        format!("  ├── embedding: {}", info.embedding_length),
        format!("  ├── context: {}", info.context_length),
        format!("  ├── tensors: {}", info.tensor_count),
        format!(
            "  └── total: {:.2} GiB",
            info.total_size_bytes as f64 / 1073741824.0
        ),
    ]
}

/// Render the tensor catalog rows for a `layer.N[.suffix]` target.
fn describe_layer(info: &vitriol_calibrate::gguf::ModelInfo, target: &str) -> Vec<String> {
    let Some(list) = matching_tensors(info, target) else {
        return vec![format!(
            "[ERR] target '{target}' — expected 'layer.N' or 'layer.N.mlp|norm|attn'"
        )];
    };
    if list.is_empty() {
        return vec![format!("[PROBE] {target} — no matching tensors")];
    }
    let mut rows: Vec<String> = list
        .iter()
        .map(|t| {
            format!(
                "  ├── {:<44} {:>10} {:>9}",
                t.name,
                vitriol_calibrate::gguf::type_name(t.ggml_type),
                human_bytes(t.size_bytes)
            )
        })
        .collect();
    let suffix_label = target
        .split('.')
        .nth(2)
        .filter(|s| !s.is_empty())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    let head = format!("[PROBE] {target}{suffix_label} — {} tensors", list.len());
    if rows.len() > 24 {
        rows.truncate(24);
        rows.push("  └── … more".into());
    }
    let mut out = vec![head];
    out.append(&mut rows);
    out
}

/// The catalog tensors matching a `layer.N[.suffix]` target. `None` when the
/// target is not a layer form.
fn matching_tensors<'a>(
    info: &'a vitriol_calibrate::gguf::ModelInfo,
    target: &str,
) -> Option<Vec<&'a vitriol_calibrate::gguf::TensorEntry>> {
    let idx = layer_index(target)?;
    let suffix = target.split('.').nth(2).unwrap_or("").to_lowercase();
    // The LARQL-style "mlp" group is the GGUF `ffn_*` tensors.
    let needle = if suffix == "mlp" { "ffn" } else { &suffix };
    let prefix = format!("blk.{idx}.");
    Some(
        info.tensors
            .iter()
            .filter(|t| {
                t.name.to_lowercase().starts_with(&prefix)
                    && (needle.is_empty() || t.name.to_lowercase().contains(needle))
            })
            .collect(),
    )
}

/// Cap on tensors processed by an aggregate CENSUS.
const CENSUS_TENSOR_CAP: usize = 64;

/// Parse `layer.N` -> layer index.
fn layer_index(target: &str) -> Option<u64> {
    let rest = target.strip_prefix("layer.")?;
    let idx = rest.split('.').next()?;
    idx.parse().ok()
}

/// Compact human size (KiB/MiB/GiB).
fn human_bytes(b: u64) -> String {
    if b >= 1 << 30 {
        return format!("{:.2} GiB", b as f64 / (1 << 30) as f64);
    }
    if b >= 1 << 20 {
        return format!("{:.1} MiB", b as f64 / (1 << 20) as f64);
    }
    if b >= 1 << 10 {
        return format!("{:.0} KiB", b as f64 / (1 << 10) as f64);
    }
    format!("{b} B")
}

/// The HELP text.
fn help_lines() -> Vec<String> {
    vec![
        "DESCRIBE > model | layer.N | layer.N.mlp    census of model/layer".into(),
        "CENSUS > layer.N.mlp                      W0 value census (dead lanes)".into(),
        "DISSOLVE > layer.N.mlp strategy   (P3) weight pruning".into(),
        "COAGULATE > layer.N norm into mlp (P3) fold normalizer".into(),
        "TEST > \"prompt\"                  run prompt through the active model".into(),
        "MAP                              print real system memory layout".into(),
        "COMPILE > \"name\"                 package .spagyr bundle".into(),
        "RECORD > \"name\"                  begin grimoire capture".into(),
        "STOP                              write the grimoire file".into(),
        "PLAY > \"name\"                    probe (or COMMIT > PLAY: run) a grimoire".into(),
        "UNDO                             revert the last committed op".into(),
        "CLEAR                            clear the output".into(),
        "Prefix a mutating command with 'COMMIT >' to apply instead of probe.".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(cfg: &'a Config, snap: &'a Snapshot) -> OpCtx<'a> {
        OpCtx {
            cfg,
            snap,
            model_path: None,
            profile: None,
        }
    }

    #[test]
    fn help_and_clear_work() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let cfg = Config::from_env();
        let snap = Snapshot::default();
        let lines = o.run("HELP", &ctx(&cfg, &snap));
        assert!(lines.len() > 5);
        o.run("CLEAR", &ctx(&cfg, &snap));
        assert!(o.output.is_empty());
    }

    #[test]
    fn unknown_command_errors() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let cfg = Config::from_env();
        let snap = Snapshot::default();
        let lines = o.run("FROBNICATE > x", &ctx(&cfg, &snap));
        assert!(lines[0].contains("[ERR]"));
    }

    #[test]
    fn dissolure_reports_p3_pending() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let cfg = Config::from_env();
        let snap = Snapshot::default();
        let lines = o.run("DISSOLVE > layer.12.mlp wanda 0.35", &ctx(&cfg, &snap));
        assert!(lines[0].contains("P3"));
        assert!(lines[0].contains("not built yet"));
    }

    #[test]
    fn record_stop_writes_grimoire() {
        let home = std::env::temp_dir().join("officina_rec_test");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let cfg = Config::from_env();
        let snap = Snapshot::default();
        o.run("RECORD > test-rec", &ctx(&cfg, &snap));
        o.record_op("op", "MAP", 0.0);
        let lines = o.run("STOP", &ctx(&cfg, &snap));
        assert!(lines[0].contains("[SAVED]"));
        assert!(grimoire::list(&home).contains(&"test-rec".to_string()));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn undo_reverts_journal_and_drift() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        o.record_op("pruned", "DISSOLVE > x", 0.0024);
        assert!((o.drift - 0.0024).abs() < 1e-9);
        let snap = Snapshot::default();
        let lines = o.run("UNDO", &ctx(&Config::from_env(), &snap));
        assert!(lines[0].contains("reverted"));
        assert!(o.journal.is_empty());
        assert!(o.drift < 1e-9);
    }

    #[test]
    fn history_nav_recalls() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        let c = ctx(&cfg, &snap);
        o.input = "MAP".into();
        o.run("MAP", &c);
        o.input = "HELP".into();
        o.run("HELP", &c);
        o.history_nav(-1);
        assert_eq!(o.input, "MAP");
    }

    #[test]
    fn syntax_check_balances() {
        assert_eq!(syntax_check("int f(){ return (1+2)*3; }"), "BALANCED");
        assert_eq!(syntax_check("int f( { return;"), "UNBALANCED");
    }

    #[test]
    fn layer_index_and_suffix_parse() {
        assert_eq!(layer_index("layer.12"), Some(12));
        assert_eq!(layer_index("layer.12.mlp"), Some(12));
        assert_eq!(layer_index("model"), None);
    }

    #[test]
    fn describe_layer_filters_catalog() {
        use vitriol_calibrate::gguf::{ModelInfo, TensorEntry};
        let info = ModelInfo {
            architecture: "qwen2".into(),
            context_length: 0,
            block_count: 0,
            expert_count: 0,
            expert_used_count: 0,
            embedding_length: 0,
            head_count: 0,
            head_count_kv: 0,
            has_mtp: false,
            total_size_bytes: 0,
            tensor_count: 0,
            per_layer_attn_bytes: 0,
            per_layer_experts_bytes: 0,
            tensors: vec![
                TensorEntry {
                    name: "blk.12.ffn_gate.weight".into(),
                    shape: vec![1, 2],
                    ggml_type: 16,
                    offset: 0,
                    size_bytes: 1024,
                },
                TensorEntry {
                    name: "blk.13.ffn_gate.weight".into(),
                    shape: vec![1, 2],
                    ggml_type: 16,
                    offset: 0,
                    size_bytes: 1024,
                },
                TensorEntry {
                    name: "blk.12.input_layernorm.weight".into(),
                    shape: vec![2],
                    ggml_type: 1,
                    offset: 0,
                    size_bytes: 512,
                },
            ],
        };
        let rows = describe_layer(&info, "layer.12");
        assert_eq!(rows.len(), 3);
        assert!(rows[0].contains("layer.12"));
        assert!(rows.iter().any(|r| r.contains("ffn_gate")));
        assert!(rows.iter().any(|r| r.contains("iq2_xxs")));
        let rows_mlp = describe_layer(&info, "layer.12.mlp");
        assert_eq!(rows_mlp.len(), 2);
        assert!(rows_mlp[1].contains("ffn_gate"));
        let rows_none = describe_layer(&info, "layer.12.attn");
        assert!(rows_none[0].contains("no matching"));
        let bad = describe_layer(&info, "model");
        assert!(bad[0].contains("[ERR]"));
    }

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(4096), "4 KiB");
        assert_eq!(human_bytes(2 * 1048576), "2.0 MiB");
    }

    #[test]
    fn matching_tensors_respects_layer_and_group() {
        use vitriol_calibrate::gguf::{ModelInfo, TensorEntry};
        let info = ModelInfo {
            architecture: "qwen2".into(),
            context_length: 0,
            block_count: 0,
            expert_count: 0,
            expert_used_count: 0,
            embedding_length: 0,
            head_count: 0,
            head_count_kv: 0,
            has_mtp: false,
            total_size_bytes: 0,
            tensor_count: 0,
            per_layer_attn_bytes: 0,
            per_layer_experts_bytes: 0,
            tensors: vec![
                TensorEntry {
                    name: "blk.0.attn_q.weight".into(),
                    shape: vec![2],
                    ggml_type: 1,
                    offset: 0,
                    size_bytes: 4,
                },
                TensorEntry {
                    name: "blk.0.ffn_gate.weight".into(),
                    shape: vec![2],
                    ggml_type: 16,
                    offset: 0,
                    size_bytes: 8,
                },
                TensorEntry {
                    name: "blk.1.attn_q.weight".into(),
                    shape: vec![2],
                    ggml_type: 1,
                    offset: 0,
                    size_bytes: 4,
                },
            ],
        };
        let all = matching_tensors(&info, "layer.0").unwrap();
        assert_eq!(all.len(), 2);
        let attn = matching_tensors(&info, "layer.0.attn").unwrap();
        assert_eq!(attn.len(), 1);
        assert!(attn[0].name.contains("attn_q"));
        let mlp = matching_tensors(&info, "layer.0.mlp").unwrap();
        assert_eq!(mlp.len(), 1);
        assert!(mlp[0].name.contains("ffn_gate"));
        assert!(matching_tensors(&info, "model").is_none());
    }

    #[test]
    fn census_keyword_is_read_only() {
        use crate::officina::grammar::Keyword;
        assert!(Keyword::Census.is_read_only());
    }
}
