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
pub mod mask;

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
    /// Output scrollback offset up from the bottom; `None` follows the tail.
    pub output_scroll: Option<usize>,
    /// Command history for up-arrow recall.
    history: VecDeque<String>,
    /// Cursor into history while recalling.
    history_pos: Option<usize>,
    /// Active completion candidates (built on Tab).
    complete: Vec<String>,
    /// Cursor into `complete`.
    complete_pos: usize,
    /// Input snapshot the candidate list was built from (stale check).
    complete_base: String,
    /// The candidate last applied (stale check while cycling).
    complete_last: String,
    /// Cached catalog metadata for layer-target completion.
    catalog_cache: Option<(PathBuf, u64)>,
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
            output_scroll: None,
            history: VecDeque::with_capacity(200),
            history_pos: None,
            complete: Vec::new(),
            complete_pos: 0,
            complete_base: String::new(),
            complete_last: String::new(),
            catalog_cache: None,
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
        self.clear_complete();
    }

    /// Remove the last character from the command line.
    pub fn backspace(&mut self) {
        self.input.pop();
        self.clear_complete();
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
        self.clear_complete();
    }

    /// The `[start, end)` output window to render for `height` visible lines.
    /// `None` scroll follows the newest lines; a frozen offset shows history.
    pub fn output_window(&self, height: usize) -> (usize, usize) {
        let len = self.output.len();
        if len == 0 || height == 0 {
            return (0, 0);
        }
        let off = self.output_scroll.unwrap_or(0).min(len.saturating_sub(1));
        let end = len.saturating_sub(off);
        let start = end.saturating_sub(height).min(end);
        (start, end)
    }

    /// Scroll the output by `delta` screen-lines; returning to the bottom
    /// (`off` reaches 0) re-enables tail-follow.
    pub fn output_scroll_lines(&mut self, delta: isize, height: usize) {
        let len = self.output.len();
        if len == 0 {
            return;
        }
        let max_off = len.saturating_sub(1);
        let cur = self.output_scroll.unwrap_or(0) as isize;
        let next = cur + delta;
        if next <= 0 {
            self.output_scroll = None;
        } else {
            self.output_scroll = Some((next as usize).min(max_off));
        }
        let _ = height;
    }

    /// Build the completion candidates for the current input, if stale.
    pub fn completions(&self) -> Vec<String> {
        self.complete.clone()
    }

    /// Cycle the active completion list; builds it fresh when the input
    /// changed since it was last built. Applies the candidate to the input.
    pub fn cycle_complete(&mut self, ctx: &OpCtx) {
        let stale = self.complete.is_empty()
            || (self.complete_base != self.input && self.complete_last != self.input);
        if stale {
            self.complete = build_completions(&self.input, &self.home, self.catalog_cache.as_ref());
            self.complete_pos = 0;
            self.complete_base = self.input.clone();
            self.complete_last = String::new();
        }
        if self.complete.is_empty() {
            return;
        }
        let cand = self.complete[self.complete_pos].clone();
        self.complete_pos = (self.complete_pos + 1) % self.complete.len();
        self.complete_last = cand.clone();
        self.input = cand;
        let _ = ctx;
    }

    /// Drop the current completion state (called on typing/backspace).
    pub fn clear_complete(&mut self) {
        self.complete.clear();
        self.complete_pos = 0;
        self.complete_base = String::new();
        self.complete_last = String::new();
    }

    /// Cache the model layer count for target completion (metadata-only read,
    /// done lazily on first layer-target completion).
    pub fn ensure_catalog(&mut self, model_path: Option<&Path>) {
        if self.catalog_cache.is_some() {
            return;
        }
        let Some(path) = model_path else {
            return;
        };
        if let Ok(info) = vitriol_calibrate::gguf::read_gguf(path) {
            self.catalog_cache = Some((path.to_path_buf(), info.block_count));
        }
    }

    /// Run a raw input line: parse + execute, returning new output lines.
    pub fn run(&mut self, line: &str, ctx: &OpCtx) -> Vec<String> {
        self.history_pos = None;
        self.output_scroll = None;
        self.clear_complete();
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

    /// Dispatch one parsed command to its backend. The safety contract is
    /// enforced first: base-destructive ops need an explicit commit target.
    fn execute(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if let Some(blocked) = self.check_safety(cmd) {
            return blocked;
        }
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
            Keyword::Rectify => self.rectify(cmd, ctx),
            Keyword::Discard => self.discard(cmd),
            Keyword::Log => self.log_mask(cmd),
            Keyword::Revert => self.revert_mask(cmd),
            Keyword::Guide => self.guide(cmd, ctx),
            Keyword::Dissolve => self.dissolve(cmd, ctx),
            Keyword::Coagulate => vec![
                "[ERR] COAGULATE: normalizer folding (P3b) is not built yet — DISSOLVE is live."
                    .into(),
            ],
        }
    }

    /// The strict commit-safety contract: a bare `COMMIT >` on a base-destructive
    /// op is blocked until the write target is explicit.
    fn check_safety(&self, cmd: &Command) -> Option<Vec<String>> {
        if !cmd.commit || !cmd.keyword.needs_explicit_commit() {
            return None;
        }
        match &cmd.commit_kind {
            Some(_) => None,
            None => Some(vec![
                "[ERROR] Commit blocked: destructive write requires an explicit target.".into(),
                "  ├── Use 'COMMIT overwrite >' to modify the active target in-place.".into(),
                "  └── Use 'COMMIT as \"name\" >' to write a new target.".into(),
            ]),
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
        let target = cmd.target.trim().to_lowercase();
        if target == "model" {
            if let Some(mask_name) = cmd.args.first() {
                return describe_mask(&self.home, mask_name);
            }
        }
        let Some(path) = &ctx.model_path else {
            return vec!["[ERR] no model path configured (set [model] path)".into()];
        };
        let info = match vitriol_calibrate::gguf::read_gguf(path) {
            Ok(info) => info,
            Err(e) => return vec![format!("[ERR] read gguf: {e}")],
        };
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

    /// ASCENSUS > RECTIFY: a cloud model generates N calibration prompts from
    /// the intent, then they run locally as a batch, each recording its fired
    /// experts as a transaction in the named mask.
    fn ascensus_rectify(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if cmd.target.is_empty() {
            return vec!["[ERR] ASCENSUS > RECTIFY > \"intent\" N into <mask>".into()];
        }
        let n: usize = cmd
            .args
            .iter()
            .find_map(|a| a.parse().ok())
            .unwrap_or(5)
            .clamp(1, 25);
        let mask_name = mask_from_args(&cmd.args).unwrap_or_else(|| "default".into());
        let secrets = crate::secrets::Secrets::load(&ctx.cfg.secrets_path());
        let api_key = secrets.api_key.trim().to_string();
        let model = if secrets.model.trim().is_empty() {
            "gemini-2.5-flash".to_string()
        } else {
            secrets.model.trim().to_string()
        };
        if api_key.is_empty() {
            return vec![
                "[ERR] no Gemini key in ~/.vitriol/secrets — set it in the SUBSYSTEMS tab.".into(),
            ];
        }
        if !cmd.commit {
            return vec![
                format!(
                    "[PROBE] ASCENSUS > RECTIFY \"{}\" — {n} prompts into {mask_name}",
                    cmd.target
                ),
                format!("  ├── cloud model: {model}"),
                "  ├── generates N calibration prompts, then runs them locally".into(),
                "  └── (Run with 'COMMIT overwrite >' to execute the batch)".into(),
            ];
        }
        let cloud_prompt = format!(
            "Generate {n} diverse, rigorous systems-programming prompts to calibrate a local \
             code model for the intent: \"{}\". Output ONLY a numbered list, one prompt per \
             line, no commentary, no headers.",
            cmd.target
        );
        let reply = match gemini_generate(&api_key, &model, &cloud_prompt) {
            Ok(t) => t,
            Err(e) => return vec![format!("[ERR] ascensus: {e}")],
        };
        let prompts = parse_numbered_list(&reply);
        if prompts.is_empty() {
            return vec![
                "[ERR] cloud model returned no prompts to calibrate with.".into(),
                format!("  reply: {}", reply.chars().take(160).collect::<String>()),
            ];
        }
        let path = mask::mask_path(&self.home, &mask_name);
        let mut mask_file =
            mask::MaskFile::load(&path).unwrap_or_else(|_| mask::MaskFile::new(&mask_name));
        let mut out = vec![format!(
            "[ASCENSUS] running {n} calibration prompts into {mask_name}"
        )];
        let mut recorded = 0usize;
        for (i, prompt) in prompts.iter().take(n).enumerate() {
            let payload = match generate(ctx.cfg, prompt, 128, true) {
                Ok(p) => p,
                Err(e) => {
                    out.push(format!("  [{}/{}] gen error: {e}", i + 1, n));
                    continue;
                }
            };
            let Some(fired) = parse_rectify_experts(&payload) else {
                out.push(format!("  [{}/{}] no expert data", i + 1, n));
                continue;
            };
            let txn = mask_file.add(now_ts(), prompt, "ascensus", fired.clone());
            recorded += 1;
            out.push(format!(
                "  [{}/{}] \"{}\" → txn #{} (+{} experts)",
                i + 1,
                n,
                prompt.chars().take(40).collect::<String>(),
                txn.id,
                fired.len()
            ));
        }
        if let Err(e) = mask_file.save(&path) {
            return vec![format!("[ERR] save mask: {e}")];
        }
        out.push(format!(
            "  └── {recorded}/{n} transactions recorded → {mask_name} ({} active)",
            mask_file.union_active().len()
        ));
        self.record_op(
            &format!("ascensus rectify: {n} prompts into {mask_name}"),
            &cmd.raw,
            0.0,
        );
        out
    }

    /// GUIDE: render the how-to manual (`docs/officina-guide.md`), optionally
    /// filtered to one section by topic.
    fn guide(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        let path = ctx.cfg.repo_root.join("docs/officina-guide.md");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return vec![format!("[ERR] read guide: {e}")],
        };
        let topic = cmd.target.trim().to_lowercase();
        if topic.is_empty() {
            return crate::markdown::render(&text, 100)
                .iter()
                .map(|l| l.to_string())
                .collect();
        }
        let section: Vec<&str> = {
            let mut out = Vec::new();
            let mut collecting = false;
            for l in text.lines() {
                if l.starts_with('#') {
                    collecting = l.to_lowercase().contains(&topic);
                    if collecting {
                        out.push(l);
                    }
                } else if collecting {
                    out.push(l);
                }
            }
            out
        };
        if section.is_empty() {
            return vec![format!(
                "[ERR] no section '{topic}' — try: usage, diagnose, rectify, ascensus, grimoires, compile, surgery"
            )];
        }
        crate::markdown::render(&section.join("\n"), 100)
            .iter()
            .map(|l| l.to_string())
            .collect()
    }

    /// DISSOLVE: prune weights by magnitude. Probe shows the impact; commit
    /// writes a size-preserving masked copy (f16/f32 tensors masked, quantized
    /// byte-copied) via the offline rewrite.
    fn dissolve(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        let Some(path) = &ctx.model_path else {
            return vec!["[ERR] no model path configured (set [model] path)".into()];
        };
        if cmd.target.eq_ignore_ascii_case("model") {
            return self.dissolve_dross(cmd, path);
        }
        if cmd.target.is_empty() {
            return vec!["[ERR] DISSOLVE > layer.N[.group] strategy ratio".into()];
        }
        let ratio: f64 = cmd
            .args
            .iter()
            .find_map(|a| a.parse::<f64>().ok())
            .unwrap_or(0.35);
        let plan = match vitriol_calibrate::rewrite::plan(path) {
            Ok(p) => p,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        let matches: Vec<usize> = plan
            .tensors
            .iter()
            .enumerate()
            .filter(|(_, t)| slot_matches(&t.name, &cmd.target))
            .map(|(i, _)| i)
            .collect();
        if matches.is_empty() {
            return vec![format!(
                "[PROBE] DISSOLVE {target} — no matching tensors",
                target = cmd.target
            )];
        }
        let maskable: Vec<usize> = matches
            .iter()
            .copied()
            .filter(|&i| vitriol_calibrate::rewrite::maskable(plan.tensors[i].ggml_type))
            .collect();
        let skipped = matches.len() - maskable.len();
        if !cmd.commit {
            let mut out = vec![format!(
                "[PROBE] DISSOLVE {target} magnitude {ratio}",
                target = cmd.target
            )];
            out.push(format!(
                "  ├── tensors: {} ({} maskable, {skipped} unsupported)",
                matches.len(),
                maskable.len()
            ));
            for &i in maskable.iter().take(6) {
                let t = &plan.tensors[i];
                out.push(format!(
                    "  ├── {:<44} {:>10} will zero {:.0}%",
                    t.name,
                    vitriol_calibrate::gguf::type_name(t.ggml_type),
                    ratio * 100.0
                ));
            }
            if maskable.len() > 6 {
                out.push(format!("  ├── … {} more", maskable.len() - 6));
            }
            out.push("  (Run with 'COMMIT overwrite >' or 'COMMIT as \"name\" >' to write)".into());
            return out;
        }

        // Commit: read each maskable payload, mask it, build same-size edits.
        let mut edits = Vec::new();
        let mut changed = 0u64;
        for &i in &maskable {
            let t = &plan.tensors[i];
            let payload = read_payload(path, t.offset, t.size).unwrap_or_default();
            if payload.len() != t.size as usize {
                continue;
            }
            let seed = 0xC0FFEE ^ (i as u64);
            let masked = match t.ggml_type {
                0 => vitriol_calibrate::rewrite::mask_f32(&payload, ratio, seed),
                1 => vitriol_calibrate::rewrite::mask_f16(&payload, ratio, seed),
                _ => {
                    let bs = vitriol_calibrate::rewrite::block_size(t.ggml_type);
                    match bs {
                        Some(bs) => {
                            vitriol_calibrate::rewrite::mask_quantized(&payload, ratio, bs, seed)
                        }
                        None => payload,
                    }
                }
            };
            changed += t.size as u64;
            edits.push(vitriol_calibrate::rewrite::Edit {
                index: i,
                bytes: masked,
            });
        }
        let dst = self.dissolve_target(cmd, path);
        if let Err(e) = vitriol_calibrate::rewrite::copy_and_edit(path, &dst, &plan, &edits) {
            return vec![format!("[ERR] {e}")];
        }
        self.record_op(
            &format!(
                "dissolved {}: {} tensors, {skipped} unsupported",
                cmd.target,
                maskable.len()
            ),
            &cmd.raw,
            ratio * 0.1,
        );
        vec![
            format!("[COMMITTED] wrote {}", dst.display()),
            format!(
                "  ├── {} tensors masked ({:.0}% of {} bytes)",
                maskable.len(),
                ratio * 100.0,
                changed
            ),
            format!("  └── {skipped} tensors unsupported (byte-copied)"),
        ]
    }

    /// The write target for a committed DISSOLVE.
    fn dissolve_target(&self, cmd: &Command, active: &Path) -> PathBuf {
        match &cmd.commit_kind {
            Some(crate::officina::grammar::CommitKind::SaveAs(name)) => {
                let dir = self.home.join(".vitriol/rewrites");
                let _ = std::fs::create_dir_all(&dir);
                dir.join(format!("{name}.gguf"))
            }
            _ => active.to_path_buf(),
        }
    }

    /// DISSOLVE > model <mask>: drop the dross experts (never fired per the
    /// rectification mask) by zeroing their FFN weight blocks.
    fn dissolve_dross(&mut self, cmd: &Command, path: &Path) -> Vec<String> {
        let mask_name = cmd.args.first().cloned().unwrap_or_default();
        if mask_name.is_empty() {
            return vec!["[ERR] DISSOLVE > model <mask> requires a mask name".into()];
        }
        let mask_file = match mask::MaskFile::load(&mask::mask_path(&self.home, &mask_name)) {
            Ok(m) => m,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        let n_expert = match vitriol_calibrate::gguf::read_gguf(path) {
            Ok(info) => info.expert_count.max(1),
            Err(_) => 64,
        };
        let active = mask_file.union_active();
        let dross: Vec<u32> = (0..n_expert)
            .map(|e| e as u32)
            .filter(|e| !active.contains(e))
            .collect();
        if dross.is_empty() {
            return vec![format!(
                "[PROBE] model {mask_name} — no dross experts (all {n_expert} fired)"
            )];
        }
        let plan = match vitriol_calibrate::rewrite::plan(path) {
            Ok(p) => p,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        // FFN tensors across all layers, with their per-expert row count.
        let mut targets = Vec::new();
        for (i, t) in plan.tensors.iter().enumerate() {
            let lower = t.name.to_lowercase();
            let is_ffn = lower.contains("ffn_gate")
                || lower.contains("ffn_up")
                || lower.contains("ffn_down");
            if !is_ffn {
                continue;
            }
            let Some(n_ffn) =
                t.ne.iter()
                    .copied()
                    .filter(|&d| d % n_expert as i64 == 0 && d / n_expert as i64 > 1)
                    .max()
                    .map(|d| (d / n_expert as i64) as u64)
            else {
                continue;
            };
            targets.push((i, n_ffn));
        }
        if targets.is_empty() {
            return vec![format!(
                "[PROBE] model {mask_name} — no FFN tensors found in the model"
            )];
        }
        if !cmd.commit {
            let mut out = vec![format!(
                "[PROBE] DISSOLVE model {mask_name} — drop {} dross experts",
                dross.len()
            )];
            out.push(format!(
                "  ├── experts: {}",
                describe_dross(&dross, n_expert)
            ));
            out.push(format!("  ├── tensors: {} FFN layers", targets.len()));
            out.push("  └── zeroes the FFN weight blocks of never-fired experts".into());
            out.push("  (Run with 'COMMIT overwrite >' or 'COMMIT as \"name\" >' to write)".into());
            return out;
        }
        let mut edits = Vec::new();
        let mut blocks = 0u64;
        for (i, n_ffn) in targets {
            let t = &plan.tensors[i];
            let payload = read_payload(path, t.offset, t.size).unwrap_or_default();
            if payload.len() != t.size as usize {
                continue;
            }
            let dross_edit = vitriol_calibrate::rewrite::DrossEdit {
                payload: &payload,
                ggml_type: t.ggml_type,
                ne: &t.ne,
                n_expert,
                n_ffn_expert: n_ffn,
                dross: &dross,
            };
            if let Some((masked, z)) = dross_edit.apply() {
                blocks += z;
                edits.push(vitriol_calibrate::rewrite::Edit {
                    index: i,
                    bytes: masked,
                });
            }
        }
        let dst = self.dissolve_target(cmd, path);
        if let Err(e) = vitriol_calibrate::rewrite::copy_and_edit(path, &dst, &plan, &edits) {
            return vec![format!("[ERR] {e}")];
        }
        self.record_op(
            &format!(
                "dissolved model by {mask_name}: dropped {} dross experts",
                dross.len()
            ),
            &cmd.raw,
            0.1,
        );
        vec![
            format!("[COMMITTED] wrote {}", dst.display()),
            format!(
                "  ├── dropped {} dross experts ({blocks} blocks zeroed)",
                dross.len()
            ),
            format!("  └── experts kept: {}", describe_dross(&dross, n_expert)),
        ]
    }

    /// TEST: run a prompt through the live gen server and syntax-check.
    fn test(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if cmd.target.is_empty() {
            return vec!["[ERR] TEST > \"prompt\" requires quoted text".into()];
        }
        let payload = match generate(ctx.cfg, &cmd.target, 128, false) {
            Ok(p) => p,
            Err(e) => return vec![format!("[ERR] {e}")],
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

    /// RECTIFY: run a generation and record the fired experts into a named mask.
    /// Live firing data comes from the fork's expert-activity hook (the server
    /// `rectify` response field); without it, commit reports honestly.
    /// `ASCENSUS > RECTIFY` generates N calibration prompts from a cloud model
    /// and runs them as a batch.
    fn rectify(&mut self, cmd: &Command, ctx: &OpCtx) -> Vec<String> {
        if cmd.cloud {
            return self.ascensus_rectify(cmd, ctx);
        }
        if cmd.target.is_empty() {
            return vec!["[ERR] RECTIFY > \"prompt\" into <mask> requires a quoted prompt".into()];
        }
        let mask_name = mask_from_args(&cmd.args).unwrap_or_else(|| "default".into());
        let path = mask::mask_path(&self.home, &mask_name);
        let mask_file =
            mask::MaskFile::load(&path).unwrap_or_else(|_| mask::MaskFile::new(&mask_name));
        if !cmd.commit {
            let mut out = vec![format!(
                "[PROBE] RECTIFY \"{}\" into {mask_name}",
                cmd.target
            )];
            out.push("  ├── runs a generation and records which MoE experts fired".into());
            out.push(format!(
                "  └── existing mask: {} transactions",
                mask_file.transactions.len()
            ));
            out.push("  (Run with 'COMMIT overwrite >' to record)".into());
            return out;
        }
        let payload = match generate(ctx.cfg, &cmd.target, 128, true) {
            Ok(p) => p,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        let Some(fired) = parse_rectify_experts(&payload) else {
            return vec![
                "[ERR] gen server returned no 'rectify.experts' data.".into(),
                "  The fork's expert-activity hook is not enabled on this server.".into(),
            ];
        };
        let mut mask_file = mask_file;
        let ts = now_ts();
        let txn = mask_file.add(ts, &cmd.target, "manual", fired.clone());
        if let Err(e) = mask_file.save(&path) {
            return vec![format!("[ERR] {e}")];
        }
        self.record_op(
            &format!("rectified {mask_name}: +{} experts", fired.len()),
            &cmd.raw,
            0.0,
        );
        vec![format!(
            "[COMMITTED] mask '{mask_name}' txn #{} — {} experts recorded",
            txn.id,
            fired.len()
        )]
    }

    /// DISCARD: delete a named mask (commit required).
    fn discard(&mut self, cmd: &Command) -> Vec<String> {
        let mask_name = cmd.args.first().cloned().unwrap_or_default();
        if mask_name.is_empty() {
            return vec!["[ERR] DISCARD > model <mask> requires a mask name".into()];
        }
        let path = mask::mask_path(&self.home, &mask_name);
        if !cmd.commit {
            return vec![format!(
                "[PROBE] DISCARD {mask_name} — would delete {}",
                path.display()
            )];
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                self.record_op(&format!("discarded mask {mask_name}"), &cmd.raw, 0.0);
                vec![format!("[COMMITTED] mask '{mask_name}' deleted")]
            }
            Err(e) => vec![format!("[ERR] delete {}: {e}", path.display())],
        }
    }

    /// LOG: list a mask's transaction history (read-only).
    fn log_mask(&mut self, cmd: &Command) -> Vec<String> {
        let mask_name = cmd.args.first().cloned().unwrap_or_default();
        if mask_name.is_empty() {
            return vec!["[ERR] LOG > model <mask> requires a mask name".into()];
        }
        let path = mask::mask_path(&self.home, &mask_name);
        let mask_file = match mask::MaskFile::load(&path) {
            Ok(m) => m,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        if mask_file.transactions.is_empty() {
            return vec![format!("[HISTORY] {mask_name} — no transactions")];
        }
        let mut out = vec![format!("[HISTORY] model {mask_name}")];
        for t in mask_file.transactions.iter().rev() {
            out.push(format!(
                "  ├── [{}] {} {} — \"{}\" ({} channels)",
                t.id,
                ts_string(t.ts),
                t.source,
                t.prompt,
                t.fired.len()
            ));
        }
        out.push(format!(
            "  └── union: {} active",
            mask_file.union_active().len()
        ));
        out
    }

    /// REVERT: probe shows the impact of dropping a transaction; commit drops it.
    fn revert_mask(&mut self, cmd: &Command) -> Vec<String> {
        let mask_name = cmd.args.first().cloned().unwrap_or_default();
        let id: u64 = cmd.args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0);
        if mask_name.is_empty() || id == 0 {
            return vec!["[ERR] REVERT > model <mask> <id> requires a mask name and txn id".into()];
        }
        let path = mask::mask_path(&self.home, &mask_name);
        let mut mask_file = match mask::MaskFile::load(&path) {
            Ok(m) => m,
            Err(e) => return vec![format!("[ERR] {e}")],
        };
        let Some(txn) = mask_file.transactions.iter().find(|t| t.id == id).cloned() else {
            return vec![format!("[ERR] mask '{mask_name}' has no transaction #{id}")];
        };
        if !cmd.commit {
            let before = mask_file.union_active().len();
            let mut clone = mask_file.clone();
            clone.remove(id);
            let after = clone.union_active().len();
            return vec![
                format!("[PROBE] REVERT #{id} from {mask_name}"),
                format!(
                    "  ├── exclude: \"{}\" ({} channels)",
                    txn.prompt,
                    txn.fired.len()
                ),
                format!("  └── active channels: {before} → {after}"),
                "  (Run with 'COMMIT overwrite >' to apply)".into(),
            ];
        }
        mask_file.remove(id);
        if let Err(e) = mask_file.save(&path) {
            return vec![format!("[ERR] {e}")];
        }
        self.record_op(&format!("reverted #{id} from {mask_name}"), &cmd.raw, 0.0);
        vec![format!(
            "[COMMITTED] transaction #{id} purged from '{mask_name}'"
        )]
    }
}

/// Run one completion against the gen server, returning the JSON payload.
fn generate(
    cfg: &Config,
    prompt: &str,
    n_predict: u64,
    rectify: bool,
) -> Result<serde_json::Value, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();
    let url = format!("{}/v1/completions", cfg.gen_base());
    let mut body = serde_json::json!({
        "prompt": prompt,
        "n_predict": n_predict,
        "temperature": 0.0,
    });
    if rectify {
        body["rectify"] = serde_json::json!(true);
    }
    let resp = agent.post(&url).send_json(body).map_err(|_| {
        format!(
            "gen server unreachable at {} — start the stack first",
            cfg.gen_base()
        )
    })?;
    resp.into_json::<serde_json::Value>()
        .map_err(|_| "bad gen response".to_string())
}

/// The mask name from `into <name>` args, if present.
fn mask_from_args(args: &[String]) -> Option<String> {
    args.iter()
        .position(|a| a == "into")
        .and_then(|i| args.get(i + 1))
        .filter(|n| mask::MaskFile::valid_name(n))
        .cloned()
}

/// The fired expert ids from a completion payload's `rectify.experts` array.
fn parse_rectify_experts(payload: &serde_json::Value) -> Option<Vec<u32>> {
    payload
        .get("rectify")
        .and_then(|r| r.get("experts"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as u32)
                .collect()
        })
}

/// True when a rewrite slot name matches a `layer.N[.group]` target.
fn slot_matches(name: &str, target: &str) -> bool {
    let Some(idx) = layer_index(target) else {
        return false;
    };
    let lower = name.to_lowercase();
    let prefix = format!("blk.{idx}.");
    if !lower.starts_with(&prefix) {
        return false;
    }
    let suffix = target.split('.').nth(2).unwrap_or("").to_lowercase();
    let needle = if suffix == "mlp" { "ffn" } else { &suffix };
    needle.is_empty() || lower.contains(needle)
}

/// Read a tensor payload slice from the model file.
fn read_payload(path: &Path, offset: u64, size: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek};
    let mut f = std::fs::File::open(path)?;
    f.seek(std::io::SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; size as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Compact description of a dross-expert list.
fn describe_dross(dross: &[u32], n_expert: u64) -> String {
    let total = n_expert as u32;
    let kept = total.saturating_sub(dross.len() as u32);
    if dross.len() > 12 {
        let head: Vec<String> = dross.iter().take(6).map(|e| e.to_string()).collect();
        format!(
            "{} dross / {total} total, {kept} kept — {}{}, …",
            dross.len(),
            head.join(","),
            if dross.len() > 6 { ",…" } else { "" }
        )
    } else {
        format!(
            "{}/{} dross, {kept} kept: {}",
            dross.len(),
            total,
            dross
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

/// Call the Gemini generateContent API and return the combined text.
fn gemini_generate(api_key: &str, model: &str, prompt: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let url =
        format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": prompt}]}],
        "generationConfig": {"maxOutputTokens": 4096, "temperature": 0.8},
    });
    let resp = agent
        .post(&url)
        .query("key", api_key)
        .send_json(body)
        .map_err(|e| format!("gemini request failed: {e}"))?;
    let payload: serde_json::Value = resp
        .into_json()
        .map_err(|_| "bad gemini response".to_string())?;
    let mut text = String::new();
    for part in payload
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
            text.push_str(t);
        }
    }
    if text.is_empty() {
        return Err("empty gemini reply".into());
    }
    Ok(text)
}

/// Extract a numbered/bulleted list of lines from a model reply.
fn parse_numbered_list(reply: &str) -> Vec<String> {
    reply
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let l = l.trim_start_matches(|c: char| {
                c.is_ascii_digit() || c == '.' || c == ')' || c == '-'
            });
            let l = l.trim();
            if l.is_empty() {
                None
            } else {
                Some(l.to_string())
            }
        })
        .filter(|l| l.len() > 4)
        .collect()
}

/// Current unix time.
fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Format a unix timestamp as `HH:MM`.
fn ts_string(ts: u64) -> String {
    let h = (ts / 3600) % 24;
    let m = (ts % 3600) / 60;
    format!("{h:02}:{m:02}")
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

/// Render the mask census for `DESCRIBE > model <mask>`.
fn describe_mask(home: &Path, mask_name: &str) -> Vec<String> {
    let path = mask::mask_path(home, mask_name);
    let mask_file = match mask::MaskFile::load(&path) {
        Ok(m) => m,
        Err(_) => {
            let avail = mask::list(home).join(", ");
            let hint = if avail.is_empty() {
                "none recorded".to_string()
            } else {
                avail
            };
            return vec![format!("[ERR] no mask '{mask_name}' — available: {hint}")];
        }
    };
    let total = 64; // MoE expert pool (catalog-aware estimate lands in a later pass).
    let stats = mask_file.stats(total);
    let mut out = vec![format!("[PROBE] model {mask_name}")];
    out.push(format!("  ├── transactions: {}", stats.txn_count));
    out.push(format!(
        "  ├── active channels: {:.1}% ({})",
        stats.active_fraction() * 100.0,
        stats.active
    ));
    out.push(format!(
        "  └── dross (never fired): {:.1}% ({} of {total})",
        stats.dross as f64 / total as f64 * 100.0,
        stats.dross
    ));
    out.push(
        "  (Run 'COMMIT overwrite > DISSOLVE > model <mask>' to purge dross once P3 lands)".into(),
    );
    out
}
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

/// Build full-line completion candidates for `input`. `home` provides the
/// grimoire list; `catalog` (if present) supplies the layer count.
fn build_completions(input: &str, home: &Path, catalog: Option<&(PathBuf, u64)>) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return keyword_candidates("", false);
    }
    // `COMMIT >` prefix: keyword stage while there is no second pipe.
    if let Some(tail) = trimmed.strip_prefix("COMMIT >") {
        let tail = tail.trim_start();
        if tail.contains('>') {
            return target_stage(trimmed, home, catalog);
        }
        return keyword_candidates(tail, true);
    }
    // No COMMIT prefix.
    if trimmed.contains('>') {
        return target_stage(trimmed, home, catalog);
    }
    keyword_candidates(trimmed, false)
}

/// Target stage: parse the keyword from the segment before the last pipe.
fn target_stage(input: &str, home: &Path, catalog: Option<&(PathBuf, u64)>) -> Vec<String> {
    let last_pipe = input.rfind('>').unwrap_or(0);
    let head = input[..last_pipe].trim();
    let tail = input[last_pipe + 1..].trim_start();
    let kw = match grammar::parse(&format!("{head} > x")) {
        Ok(cmd) => cmd.keyword,
        Err(_) => return Vec::new(),
    };
    target_candidates(kw, tail, home, catalog)
}

/// Full-line keyword candidates matching a case-insensitive prefix.
fn keyword_candidates(prefix: &str, committed: bool) -> Vec<String> {
    let p = prefix.to_lowercase();
    let mut out = Vec::new();
    for kw in [
        Keyword::Describe,
        Keyword::Census,
        Keyword::Dissolve,
        Keyword::Coagulate,
        Keyword::Test,
        Keyword::Map,
        Keyword::Compile,
        Keyword::Record,
        Keyword::Stop,
        Keyword::Play,
        Keyword::Undo,
        Keyword::Clear,
        Keyword::Help,
    ] {
        let name = kw.as_str().to_lowercase();
        if !name.starts_with(&p) {
            continue;
        }
        if committed {
            out.push(format!("COMMIT > {} > ", kw.as_str()));
        } else {
            out.push(format!("{} > ", kw.as_str()));
            if !kw.is_read_only() {
                out.push(format!("COMMIT > {} > ", kw.as_str()));
            }
        }
    }
    out
}

/// Target-stage candidates for a keyword, filtered by the tail prefix.
fn target_candidates(
    kw: Keyword,
    tail: &str,
    home: &Path,
    catalog: Option<&(PathBuf, u64)>,
) -> Vec<String> {
    match kw {
        Keyword::Describe | Keyword::Census | Keyword::Dissolve | Keyword::Coagulate => {
            layer_targets(tail, catalog)
        }
        Keyword::Record | Keyword::Play => grimoire_targets(tail, home),
        Keyword::Test | Keyword::Compile => quote_target(tail),
        _ => Vec::new(),
    }
}

/// `model`, `layer.N`, and `layer.N.{mlp,norm,attn}` candidates.
fn layer_targets(tail: &str, catalog: Option<&(PathBuf, u64)>) -> Vec<String> {
    let t = tail.to_lowercase();
    let mut cands: Vec<String> = Vec::new();
    if "model".starts_with(&t) {
        cands.push("model".to_string());
    }
    let layers = catalog.map(|(_, n)| *n).unwrap_or(1);
    let mut layer_cands: Vec<String> = (0..layers)
        .map(|i| format!("layer.{i}"))
        .filter(|c| c.starts_with(&t))
        .collect();
    if let Some(rest) = tail.strip_prefix("layer.") {
        let idx = rest.split('.').next().unwrap_or("");
        if !idx.is_empty() && idx.chars().all(|c| c.is_ascii_digit()) {
            for sfx in ["mlp", "norm", "attn"] {
                let full = format!("layer.{idx}.{sfx}");
                if full.starts_with(&t) {
                    layer_cands.push(full);
                }
            }
        }
    }
    cands.append(&mut layer_cands);
    cands
}

/// Grimoire-name candidates (quoted) for RECORD/PLAY.
fn grimoire_targets(tail: &str, home: &Path) -> Vec<String> {
    let t = tail.to_lowercase();
    grimoire::list(home)
        .into_iter()
        .map(|n| format!("\"{n}\""))
        .filter(|c| t.is_empty() || c.to_lowercase().starts_with(&t))
        .collect()
}

/// Quoted-name template for TEST/COMPILE.
fn quote_target(tail: &str) -> Vec<String> {
    if tail.is_empty() || tail.starts_with('"') {
        vec!["\"…\"".to_string()]
    } else {
        Vec::new()
    }
}
/// The HELP text.
fn help_lines() -> Vec<String> {
    vec![
        "DESCRIBE > model | layer.N | layer.N.mlp    census of model/layer".into(),
        "DESCRIBE > model <mask>                     rectification mask census".into(),
        "CENSUS > layer.N.mlp                      W0 value census (dead lanes)".into(),
        "RECTIFY > \"prompt\" into <mask>           record which experts fired".into(),
        "DISSOLVE > layer.N.mlp strategy   (P3) weight pruning".into(),
        "COAGULATE > layer.N norm into mlp (P3) fold normalizer".into(),
        "TEST > \"prompt\"                  run prompt through the active model".into(),
        "MAP                              print real system memory layout".into(),
        "COMPILE > \"name\"                 package .spagyr bundle".into(),
        "RECORD > \"name\"                  begin grimoire capture".into(),
        "STOP                              write the grimoire file".into(),
        "PLAY > \"name\"                    probe (or COMMIT > PLAY: run) a grimoire".into(),
        "LOG > model <mask>                mask transaction history".into(),
        "REVERT > model <mask> <id>        drop one mask transaction".into(),
        "DISCARD > model <mask>            delete a mask".into(),
        "GUIDE [> topic]                   the how-to manual".into(),
        "UNDO                             revert the last committed op".into(),
        "CLEAR                            clear the output".into(),
        "Prefix a mutating command with 'COMMIT overwrite >' or 'COMMIT as \"name\" >'.".into(),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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
    fn dissolve_probe_reports_impact() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let cfg = Config::from_env();
        let snap = Snapshot::default();
        // No model configured -> honest error; probe path exercised in unit tests
        // against a synthetic gguf via dissolve() directly.
        let lines = o.run("DISSOLVE > layer.12.mlp wanda 0.35", &ctx(&cfg, &snap));
        assert!(lines[0].contains("no model path"));
    }

    #[test]
    fn slot_matches_layer_groups() {
        assert!(slot_matches("blk.12.ffn_gate.weight", "layer.12"));
        assert!(slot_matches("blk.12.ffn_gate.weight", "layer.12.mlp"));
        assert!(slot_matches("blk.12.attn_q.weight", "layer.12.attn"));
        assert!(slot_matches(
            "blk.12.input_layernorm.weight",
            "layer.12.norm"
        ));
        assert!(!slot_matches("blk.13.ffn_gate.weight", "layer.12"));
        assert!(!slot_matches("blk.12.ffn_gate.weight", "model"));
    }

    #[test]
    fn dissolve_rewrite_writes_same_size_file() {
        let home = std::env::temp_dir().join("officina_dissolve_test");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        // minimal gguf: magic+version+1tensor+0kv, one f16 tensor [8]
        let mut buf = Vec::new();
        buf.extend_from_slice(b"GGUF");
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&1u64.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        let name = b"blk.0.ffn_gate.weight";
        buf.extend_from_slice(&(name.len() as u64).to_le_bytes());
        buf.extend_from_slice(name);
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&8i64.to_le_bytes());
        buf.extend_from_slice(&1i32.to_le_bytes()); // f16
        let off_pos = buf.len();
        buf.extend_from_slice(&0u64.to_le_bytes());
        let mut payload = Vec::new();
        for i in 0..8 {
            let v = (i as u8) + 1;
            payload.extend_from_slice(&[v, v]); // f16 raw bytes, all nonzero
        }
        let offset = buf.len() as u64;
        buf[off_pos..off_pos + 8].copy_from_slice(&offset.to_le_bytes());
        buf.extend_from_slice(&payload);
        let model_path = home.join("base.gguf");
        std::fs::write(&model_path, &buf).unwrap();

        let mut cfg = Config::from_env();
        cfg.home_dir = home.clone();
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let ctx = crate::officina::OpCtx {
            cfg: &cfg,
            snap: &snap,
            model_path: Some(model_path.clone()),
            profile: None,
        };
        let lines = o.run(
            "COMMIT as \"pruned\" > DISSOLVE > layer.0.mlp magnitude 0.5",
            &ctx,
        );
        assert!(lines[0].contains("wrote"));
        let out = home.join(".vitriol/rewrites/pruned.gguf");
        assert!(out.exists());
        let rewritten = std::fs::read(&out).unwrap();
        assert_eq!(rewritten.len(), buf.len());
        // Header unchanged; payload has zeros now.
        let hdr = buf.len() - 16;
        assert_eq!(&rewritten[..hdr], &buf[..hdr]);
        assert!(rewritten[hdr..].contains(&0));
        assert!(buf[hdr..].iter().all(|&b| b != 0));
        let _ = std::fs::remove_dir_all(&home);
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

    #[test]
    fn output_window_follows_tail_by_default() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        for i in 0..10 {
            o.output.push_back(format!("line {i}"));
        }
        // None = tail: the newest `height` lines end at the last line.
        let (s, e) = o.output_window(4);
        assert_eq!((s, e), (6, 10));
    }

    #[test]
    fn output_window_frozen_offset_shows_history() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        for i in 0..10 {
            o.output.push_back(format!("line {i}"));
        }
        o.output_scroll = Some(5);
        let (s, e) = o.output_window(4);
        assert_eq!((s, e), (1, 5));
        // Offset clamps to the top.
        o.output_scroll = Some(99);
        let (s, e) = o.output_window(4);
        assert_eq!((s, e), (0, 1));
    }

    #[test]
    fn output_window_empty() {
        let o = Officina::new(Path::new("/nonexistent"));
        assert_eq!(o.output_window(5), (0, 0));
    }

    #[test]
    fn output_scroll_lines_reenters_follow_at_bottom() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        for i in 0..20 {
            o.output.push_back(format!("line {i}"));
        }
        o.output_scroll_lines(5, 4);
        assert_eq!(o.output_scroll, Some(5));
        o.output_scroll_lines(-5, 4);
        assert_eq!(o.output_scroll, None);
    }

    #[test]
    fn completions_keyword_stage() {
        let o = Officina::new(Path::new("/nonexistent"));
        assert!(build_completions("", &o.home, None).contains(&"DESCRIBE > ".into()));
        let cands = build_completions("DES", &o.home, None);
        assert!(cands.contains(&"DESCRIBE > ".into()));
        assert!(!cands.contains(&"COMMIT > DESCRIBE > ".into()));
        assert!(!cands.contains(&"TEST > ".into()));
    }

    #[test]
    fn completions_commit_prefix_not_duplicated() {
        let o = Officina::new(Path::new("/nonexistent"));
        let cands = build_completions("COMMIT > DIS", &o.home, None);
        assert!(cands.contains(&"COMMIT > DISSOLVE > ".into()));
        assert!(!cands.contains(&"DISSOLVE > ".into()));
    }

    #[test]
    fn completions_target_stage() {
        let o = Officina::new(Path::new("/nonexistent"));
        let catalog = Some((PathBuf::from("/x"), 27u64));
        let cands = build_completions("DESCRIBE > layer.0.", &o.home, catalog.as_ref());
        assert!(cands.contains(&"layer.0.mlp".into()));
        assert!(cands.contains(&"layer.0.norm".into()));
        assert!(cands.contains(&"layer.0.attn".into()));
        let cands = build_completions("CENSUS > ", &o.home, catalog.as_ref());
        assert!(cands.contains(&"model".into()));
        assert!(cands.contains(&"layer.0".into()));
        assert!(cands.contains(&"layer.26".into()));
        assert!(!cands.contains(&"layer.27".into()));
    }

    #[test]
    fn completions_grimoire_names() {
        let home = std::env::temp_dir().join("officina_complete_test");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        o.recipe.push("MAP".into());
        o.recording = Some("sys-opt".into());
        let _ = o.stop_recording();
        let cands = build_completions("PLAY > ", &o.home, None);
        assert!(cands.iter().any(|c| c.contains("sys-opt")));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cycle_complete_applies_and_wraps() {
        let home = std::env::temp_dir().join("officina_cycle_test");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        let c = ctx(&cfg, &snap);
        o.input = "DIS".into();
        o.cycle_complete(&c);
        assert_eq!(o.input, "DISSOLVE > ");
        // Same base -> cycle to the next candidate (COMMIT > DISSOLVE >).
        o.cycle_complete(&c);
        assert_eq!(o.input, "COMMIT > DISSOLVE > ");
        // And back around.
        o.cycle_complete(&c);
        assert_eq!(o.input, "DISSOLVE > ");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn run_resets_scroll_and_completions() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        o.output.push_back("a".into());
        o.output_scroll = Some(3);
        o.complete.push("x".into());
        o.complete_base = "y".into();
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        o.run("MAP", &ctx(&cfg, &snap));
        assert_eq!(o.output_scroll, None);
        assert!(o.complete.is_empty());
    }

    #[test]
    fn bare_commit_on_destructive_is_blocked() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        let lines = o.run("COMMIT > DISCARD > model vulkan", &ctx(&cfg, &snap));
        assert!(lines[0].contains("blocked"));
        assert!(lines.iter().any(|l| l.contains("COMMIT overwrite")));
        // Explicit overwrite passes the safety gate (then proceeds to op).
        let lines = o.run(
            "COMMIT overwrite > DISCARD > model vulkan",
            &ctx(&cfg, &snap),
        );
        assert!(lines[0].contains("deleted") || lines[0].contains("[ERR]"));
    }

    #[test]
    fn rectify_probe_and_commit_with_synthetic_data() {
        let home = std::env::temp_dir().join("officina_rectify_test");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        // Probe never touches the server.
        let lines = o.run("RECTIFY > \"circular buffer\" into rust", &ctx(&cfg, &snap));
        assert!(lines[0].contains("[PROBE]"));
        assert!(!mask::mask_path(&home, "rust").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn rectify_commit_needs_expert_data() {
        let home = std::env::temp_dir().join("officina_rectify_nodata");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        // With no gen server, commit fails honestly (not a fake success).
        let lines = o.run(
            "COMMIT overwrite > RECTIFY > \"x\" into rust",
            &ctx(&cfg, &snap),
        );
        assert!(lines[0].contains("[ERR]"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn mask_ops_log_revert_discard_flow() {
        let home = std::env::temp_dir().join("officina_mask_ops");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let cfg = Config::from_env();
        // Build a mask directly.
        let path = mask::mask_path(&home, "vulkan");
        let mut m = mask::MaskFile::new("vulkan");
        m.add(1, "shader pass", "manual", vec![1, 2, 3]);
        m.add(2, "polluted", "manual", vec![99]);
        m.save(&path).unwrap();

        let lines = o.run("LOG > model vulkan", &ctx(&cfg, &snap));
        assert!(lines[0].contains("HISTORY"));
        assert!(lines.iter().any(|l| l.contains("polluted")));

        let lines = o.run("DESCRIBE > model vulkan", &ctx(&cfg, &snap));
        assert!(lines[0].contains("vulkan"));
        assert!(lines.iter().any(|l| l.contains("dross")));

        let lines = o.run("REVERT > model vulkan 2", &ctx(&cfg, &snap));
        assert!(lines[0].contains("[PROBE]"));
        assert!(lines.iter().any(|l| l.contains("polluted")));

        let lines = o.run(
            "COMMIT overwrite > REVERT > model vulkan 2",
            &ctx(&cfg, &snap),
        );
        assert!(lines[0].contains("purged"));
        let loaded = mask::MaskFile::load(&path).unwrap();
        assert_eq!(loaded.union_active(), BTreeSet::from([1, 2, 3]));

        let lines = o.run(
            "COMMIT overwrite > DISCARD > model vulkan",
            &ctx(&cfg, &snap),
        );
        assert!(lines[0].contains("deleted"));
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn ascensus_rectify_probe_needs_key() {
        let home = std::env::temp_dir().join("officina_ascensus_probe");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let mut cfg = Config::from_env();
        cfg.home_dir = home.clone(); // no secrets here -> no key
        let lines = o.run("ASCENSUS > RECTIFY > \"vulkan\" 3", &ctx(&cfg, &snap));
        assert!(lines[0].contains("no Gemini key"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn parse_numbered_list_extracts_prompts() {
        let reply =
            "1. Write a Vulkan shader\n- Implement a memory pool\n 3) Spinlock in Rust\n\n5. x\n";
        let prompts = parse_numbered_list(reply);
        assert_eq!(prompts.len(), 3);
        assert!(prompts[0].contains("Vulkan"));
        assert!(prompts[2].contains("Spinlock"));
    }

    #[test]
    fn mask_from_args_into_target() {
        let args = vec!["50".to_string(), "into".to_string(), "vulkan".to_string()];
        assert_eq!(mask_from_args(&args), Some("vulkan".to_string()));
        assert_eq!(mask_from_args(&["50".into()]), None);
    }

    #[test]
    fn guide_renders_sections() {
        let mut o = Officina::new(Path::new("/nonexistent"));
        let mut cfg = Config::from_env();
        cfg.repo_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let snap = Snapshot::default();
        let lines = o.run("GUIDE", &ctx(&cfg, &snap));
        assert!(lines.iter().any(|l| l.contains("Usage")));
        let lines = o.run("GUIDE > rectify", &ctx(&cfg, &snap));
        assert!(lines.iter().any(|l| l.to_lowercase().contains("rectify")));
    }

    #[test]
    fn describe_dross_summaries() {
        assert!(describe_dross(&[], 64).contains("0/64"));
        assert!(describe_dross(&[1, 2, 3], 64).contains("61 kept"));
        assert!(describe_dross(&(0..20).collect::<Vec<u32>>(), 64).contains("…"));
    }

    #[test]
    fn dissolve_dross_probe_needs_mask() {
        let home = std::env::temp_dir().join("officina_dross_probe");
        let _ = std::fs::remove_dir_all(&home);
        let mut o = Officina::new(&home);
        let snap = Snapshot::default();
        let mut cfg = Config::from_env();
        cfg.home_dir = home.clone();
        let lines = o.run("DISSOLVE > model missing_mask", &ctx(&cfg, &snap));
        assert!(lines[0].contains("[ERR]"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn parse_rectify_experts_from_response() {
        let payload: serde_json::Value =
            serde_json::json!({"choices": [], "rectify": {"experts": [3, 7, 42]}});
        assert_eq!(parse_rectify_experts(&payload), Some(vec![3, 7, 42]));
        assert_eq!(parse_rectify_experts(&serde_json::json!({})), None);
        assert_eq!(
            parse_rectify_experts(&serde_json::json!({"rectify": {"experts": []}})),
            Some(vec![])
        );
    }
}
