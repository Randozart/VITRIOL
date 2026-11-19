// density-bench — does constrained output actually cost fewer tokens?
//
// Built for the 2026-11-18 token-density experiments: the claim "grammar
// constrained structured lanes are 30-60 % cheaper" is testable, and this is the
// tool that tests it. Discipline is encoded, not promised:
//
//   * a run must name its arms and repeats, and always measures the baseline
//     arm first so the comparison is paired (thermal/tenant drift dominates here);
//   * every row carries the engine's own timings, so no metric is inferred;
//   * the JSON output is the artifact — the markdown table is for humans.
//
//   density-bench --url http://127.0.0.1:8279 --arm baseline --arm grammar \
//                 --shapes structured,code-edit --repeats 3 --json /tmp/e1.json
//
// Notes:
//   * `--max-tokens` caps every arm so length differences are the model's, not
//     the harness's.
//   * `--deep-sections` states the depth instead of a token count (a window is
//     not a fill); 40 sections ≈ 1.4k prompt tokens, 400 ≈ 14k.

mod arms;
mod codebook;
mod engine;
mod sample;
mod shapes;
mod sweep;

use anyhow::Result;
use arms::RequestStyle;
use engine::Client;
use sample::{median, Sample};
use shapes::Shape;

struct Args {
    url: String,
    model: String,
    arms: Vec<RequestStyle>,
    /// One style applied to every sweep arm (engine knob × request style).
    sweep_style: RequestStyle,
    shape_filter: Vec<String>,
    repeats: usize,
    max_tokens: u32,
    /// Raw JSON passed through as `chat_template_kwargs` (T1: template arms).
    template_kwargs: Option<String>,
    /// Thinking budget in tokens (T2). 0 = no thinking at all.
    think_budget: Option<u32>,
    /// Language the model is told to think in (T4): "en" (no-op) or "zh".
    reason_lang: Option<String>,
    deep_sections: usize,
    json_out: Option<String>,
    /// Sweep mode inputs (engine binary, base args, dimensions).
    engine: Option<String>,
    base_args: String,
    sets: Vec<String>,
    env_sets: Vec<String>,
    variant_specs: Vec<String>,
    dry_run: bool,
    wait_health_s: u64,
    /// Skip the streamed pass. Token metrics are contention-independent; timing is
    /// not, so on a shared engine (or while the operator is using it) measure
    /// tokens only rather than reporting timings contaminated by interference.
    no_stream: bool,
}

fn usage() -> &'static str {
    "density-bench — token-density benchmark for structured vs prose lanes

USAGE:
  density-bench [--url URL] [--arm NAME]... [--shapes LIST] [--repeats N]
                [--max-tokens N] [--deep-sections N] [--json PATH]

OPTIONS:
  --url URL            engine base URL (default http://127.0.0.1:8279)
  --model NAME         model name to report in the artifact
  --arm NAME           arm to measure; repeatable (default: baseline)
                       baseline = plain request, grammar = request + json_schema.
                       In sweep mode --arm applies that style to every engine arm,
                       so one run can cross an engine knob with a request style.
  --shapes LIST        comma-separated subset of structured,code-edit,deep-recall
  --repeats N          samples per arm per shape (default 3)
  --max-tokens N       output cap per request (default 256)
  --deep-sections N    size of the deep-recall prompt (default 40)
  --template-kwargs J  JSON sent as chat_template_kwargs, e.g. {\"preserve_thinking\": false}
                       (T1: a template variable becomes an arm without a restart)
  --think-budget N     enable thinking bounded to N tokens (T2; CROSS x N)
  --reason-lang L      tell the model to deliberate in L (en|zh) — T4, same task
  --json PATH          write the raw samples as JSON
  --no-stream          skip the TTFT pass (tokens only; use on a shared engine)
"
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        url: "http://127.0.0.1:8279".into(),
        model: "unknown".into(),
        arms: Vec::new(),
        sweep_style: RequestStyle::Baseline,
        shape_filter: Vec::new(),
        repeats: 3,
        max_tokens: 256,
        template_kwargs: None,
        think_budget: None,
        reason_lang: None,
        deep_sections: 40,
        json_out: None,
        no_stream: false,
        engine: None,
        base_args: String::new(),
        sets: Vec::new(),
        env_sets: Vec::new(),
        variant_specs: Vec::new(),
        dry_run: false,
        wait_health_s: 300,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let value = |i: usize| -> Result<String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("{} needs a value", argv[i]))
        };
        match argv[i].as_str() {
            "--url" => {
                args.url = value(i)?;
                i += 2;
            }
            "--model" => {
                args.model = value(i)?;
                i += 2;
            }
            "--arm" => {
                let name = value(i)?;
                let style = RequestStyle::from_name(&name).ok_or_else(|| {
                    anyhow::anyhow!("unknown arm {name} (known: {})", RequestStyle::names())
                })?;
                args.arms.push(style);
                args.sweep_style = style;
                i += 2;
            }
            "--shapes" => {
                args.shape_filter = value(i)?.split(',').map(|s| s.trim().to_string()).collect();
                i += 2;
            }
            "--repeats" => {
                args.repeats = value(i)?.parse()?;
                i += 2;
            }
            "--max-tokens" => {
                args.max_tokens = value(i)?.parse()?;
                i += 2;
            }
            "--deep-sections" => {
                args.deep_sections = value(i)?.parse()?;
                i += 2;
            }
            "--json" => {
                args.json_out = Some(value(i)?);
                i += 2;
            }
            "--no-stream" => {
                args.no_stream = true;
                i += 1;
            }
            "--engine" => {
                args.engine = Some(value(i)?);
                i += 2;
            }
            "--arg" => {
                args.base_args = value(i)?;
                i += 2;
            }
            "--set" => {
                args.sets.push(value(i)?);
                i += 2;
            }
            "--variant" => {
                args.variant_specs.push(value(i)?);
                i += 2;
            }
            "--env" => {
                args.env_sets.push(value(i)?);
                i += 2;
            }
            "--dry-run" => {
                args.dry_run = true;
                i += 1;
            }
            "--reason-lang" => {
                args.reason_lang = Some(value(i)?.to_string());
                i += 2;
            }
            "--think-budget" => {
                args.think_budget = Some(value(i)?.parse()?);
                i += 2;
            }
            "--template-kwargs" => {
                args.template_kwargs = Some(value(i)?.to_string());
                i += 2;
            }
            "--wait-health" => {
                args.wait_health_s = value(i)?.parse()?;
                i += 2;
            }
            "sweep" => {
                i += 1;
            }
            "-h" | "--help" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other} (try --help)"),
        }
    }
    if args.arms.is_empty() {
        args.arms.push(RequestStyle::Baseline);
        args.sweep_style = RequestStyle::Baseline;
    }
    if args.repeats == 0 {
        anyhow::bail!("--repeats must be at least 1");
    }
    Ok(args)
}

impl Args {
    /// System-message suffix for the reasoning-language arm. `en` is a no-op so
    /// the control arm is byte-identical to a plain run.
    fn system_append(&self) -> Option<String> {
        match self.reason_lang.as_deref() {
            None | Some("en") => None,
            Some("zh") => Some("Think in Chinese 中文 before answering; the final answer must still be the integer only.".to_string()),
            Some(other) => Some(format!("Think in {other} before answering.")),
        }
    }

    /// Parse `--template-kwargs` once, at the boundary: a typo in the JSON must
    /// fail the run, not silently send no kwargs and make two arms identical.
    fn template_kwargs_json(&self) -> Result<Option<serde_json::Value>> {
        match &self.template_kwargs {
            None => Ok(None),
            Some(raw) => Ok(Some(serde_json::from_str(raw).map_err(|e| {
                anyhow::anyhow!("--template-kwargs is not valid JSON: {e}")
            })?)),
        }
    }
}

/// Fingerprint line the engine prints at load, so a result is always attributable.
fn fingerprint(url: &str) -> String {
    let props = format!("{}/props", url.trim_end_matches('/'));
    match ureq::get(&props)
        .timeout(std::time::Duration::from_secs(5))
        .call()
    {
        Ok(res) => match res.into_json::<serde_json::Value>() {
            Ok(v) => {
                let model = v["model_path"].as_str().unwrap_or("?");
                let ctx = v["default_generation_settings"]["n_ctx"]
                    .as_u64()
                    .or_else(|| v["n_ctx"].as_u64())
                    .unwrap_or(0);
                format!("props model={model} n_ctx={ctx}")
            }
            Err(_) => "props unreadable".to_string(),
        },
        Err(e) => format!("props unreachable ({e})"),
    }
}

/// Run one engine per arm, measure the same shapes, report per-arm medians.
///
/// Each arm gets its own process and its own log, and is killed before the next
/// starts — two engines on one GPU would measure contention, not configuration.
fn run_sweep(args: &Args, shapes: &[&Shape]) -> Result<()> {
    let engine = args
        .engine
        .clone()
        .ok_or_else(|| anyhow::anyhow!("sweep needs --engine <path to llama-server>"))?;
    // A named variant states its flag words, so knobs that need two flags at once
    // (spec-type + draft depth) stay one arm instead of a bogus cross-product.
    let arms = if args.variant_specs.is_empty() {
        sweep::matrix(&args.sets, &args.env_sets)
    } else {
        sweep::variants(&args.variant_specs)
    };
    let mut samples: Vec<Sample> = Vec::new();
    println!("density-bench sweep · {} arm(s)", arms.len());
    for (i, arm) in arms.iter().enumerate() {
        println!("  arm {i}: {}", arm.label());
        println!(
            "         argv: {}",
            arm.argv(&args.base_args, &engine).join(" ")
        );
    }
    if args.dry_run {
        println!("  dry run — nothing started");
        return Ok(());
    }
    for (i, arm) in arms.iter().enumerate() {
        let argv = arm.argv(&args.base_args, &engine);
        println!("\n  ── arm {i}: {} ──", arm.label());
        let log_path = format!("/tmp/density-sweep-arm{i}.log");
        let log = std::fs::File::create(&log_path)?;
        let mut command = std::process::Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .stdout(std::process::Stdio::from(log.try_clone()?))
            .stderr(std::process::Stdio::from(log));
        for (k, v) in &arm.env {
            command.env(k, v);
        }
        let mut child = command.spawn()?;
        if !wait_healthy(&args.url, args.wait_health_s) {
            eprintln!("  arm {i} never became healthy — see {log_path}");
            let _ = child.kill();
            let _ = child.wait();
            continue;
        }
        let client = Client {
            base: args.url.clone(),
            model: args.model.clone(),
            max_tokens: args.max_tokens,
            template_kwargs: args.template_kwargs_json()?,
            think_budget: args.think_budget,
            system_append: args.system_append(),
            stream: !args.no_stream,
        };
        for shape in shapes {
            for run in 0..args.repeats {
                let label = sweep_label(&arm.label(), args.sweep_style);
                match client.run(shape, &label, args.sweep_style) {
                    Ok(s) => {
                        samples.push(s.clone());
                        println!(
                            "  {:<12} run {} · decode {:>6.2} tok/s · {} tok{}{}",
                            shape.id,
                            run + 1,
                            s.timings.decode_tps(),
                            s.timings.predicted_n,
                            match s.valid {
                                Some(true) => " · valid",
                                Some(false) => " · INVALID",
                                None => "",
                            },
                            match s.recall_ok {
                                Some(true) => " · recalled",
                                Some(false) => " · RECALL LOST",
                                None => "",
                            }
                        )
                    }
                    Err(e) => eprintln!("  {:<12} run {} failed: {e:#}", shape.id, run + 1),
                }
            }
        }
        print_eviction_metrics(&args.url);
        let _ = child.kill();
        let _ = child.wait();
    }
    write_artifact(args, &samples)?;
    println!("\n  sweep complete — logs under /tmp/density-sweep-arm*.log");
    Ok(())
}

/// I3: an arm's recorded label names the engine knobs *and* the request style.
///
/// Before the refactor `sweep` handed its engine label to the request builder as
/// if it were a style, so "grammar" matched nothing and the report lied.
fn sweep_label(engine_arm: &str, style: RequestStyle) -> String {
    format!("{engine_arm} · {}", style.name())
}

#[cfg(test)]
mod sweep_label_tests {
    use super::*;

    #[test]
    fn i3_the_label_names_both_axes() {
        let label = sweep_label("VITRIOL_KV_SCORE_EVERY=8", RequestStyle::Grammar);
        assert!(label.contains("VITRIOL_KV_SCORE_EVERY=8"));
        assert!(label.ends_with("· grammar"));
        assert_eq!(sweep_label("", RequestStyle::Baseline), " · baseline");
    }
}

/// Print the arm's eviction counters.
///
/// E6's whole question is what eviction DID, not only how fast the answer came
/// back: a sweep that never ejected anything measured nothing. Failures print a
/// note rather than aborting the arm.
fn print_eviction_metrics(url: &str) {
    let endpoint = format!("{}/metrics", url.trim_end_matches('/'));
    let Ok(res) = ureq::get(&endpoint)
        .timeout(std::time::Duration::from_secs(5))
        .call()
    else {
        println!("  metrics unavailable at {endpoint}");
        return;
    };
    let body = res.into_string().unwrap_or_default();
    for line in body.lines() {
        if line.contains("kv_ejected_total") || line.contains("kv_cache_bytes") {
            println!("  {line}");
        }
    }
}

/// Poll `/health` until the engine answers or the deadline passes.
fn wait_healthy(url: &str, seconds: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let health = format!("{}/health", url.trim_end_matches('/'));
    while std::time::Instant::now() < deadline {
        if let Ok(res) = ureq::get(&health)
            .timeout(std::time::Duration::from_secs(3))
            .call()
        {
            if res.status() == 200 {
                return true;
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    false
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let client = Client {
        base: args.url.clone(),
        model: args.model.clone(),
        max_tokens: args.max_tokens,
        template_kwargs: args.template_kwargs_json()?,
        think_budget: args.think_budget,
        system_append: args.system_append(),
        stream: !args.no_stream,
    };
    let all = shapes::all(args.deep_sections);
    let shapes: Vec<&Shape> = if args.shape_filter.is_empty() {
        all.iter().collect()
    } else {
        all.iter()
            .filter(|s| args.shape_filter.iter().any(|f| f == s.id))
            .collect()
    };
    if shapes.is_empty() {
        anyhow::bail!("no shapes selected — check --shapes");
    }

    if args.engine.is_some() {
        return run_sweep(&args, &shapes);
    }

    println!("density-bench");
    println!("  engine : {} · model {}", args.url, client.model);
    println!("  fingerprint: {}", fingerprint(&args.url));
    println!(
        "  arms   : {} · repeats {} · max_tokens {} · deep_sections {}",
        args.arms
            .iter()
            .map(|a| a.name())
            .collect::<Vec<_>>()
            .join(" + "),
        args.repeats,
        args.max_tokens,
        args.deep_sections
    );

    let mut samples: Vec<Sample> = Vec::new();
    for shape in &shapes {
        // Paired measurement: the baseline is re-measured next to every arm, in
        // the same instant, because drift on this box is larger than most effects.
        let mut arms = args.arms.clone();
        if !arms.contains(&RequestStyle::Baseline) {
            arms.insert(0, RequestStyle::Baseline);
        }
        println!("  shape  : {} ({})", shape.id, shape.note);
        for arm in &arms {
            for run in 0..args.repeats {
                match client.run(shape, arm.name(), *arm) {
                    Ok(s) => {
                        println!(
                            "  {:<12} {:<9} run {} · prompt {:>7.1} tok/s · decode {:>6.2} tok/s · \
                             ttft {:>6.2}s · {} chars{}{}{}{}",
                            shape.id,
                            arm.name(),
                            run + 1,
                            s.timings.prompt_tps(),
                            s.timings.decode_tps(),
                            s.ttft_s,
                            s.content_chars,
                            match s.valid {
                                Some(true) => " · valid",
                                Some(false) => " · INVALID",
                                None => "",
                            },
                            match s.recall_ok {
                                Some(true) => " · recalled",
                                Some(false) => " · RECALL LOST",
                                None => "",
                            },
                            // A truncated answer is not a short answer: say so.
                            match (&s.content_chars, &s.finish_reason) {
                                (0, Some(f)) => format!(" · EMPTY ANSWER (finish={f})"),
                                (_, Some(f)) if f != "stop" => format!(" · finish={f}"),
                                _ => String::new(),
                            },
                            match (&s.expanded_chars, &s.invented) {
                                (_, Some(sym)) => format!(" · INVENTED {sym:?}"),
                                (Some(n), None) => format!(" · decoded {n} chars"),
                                (None, None) => String::new(),
                            }
                        );
                        samples.push(s);
                    }
                    Err(e) => {
                        eprintln!(
                            "  {:<12} {:<9} run {} failed: {e:#}",
                            shape.id,
                            arm.name(),
                            run + 1
                        )
                    }
                }
            }
        }
    }

    summarize(&samples);
    write_artifact(&args, &samples)?;
    Ok(())
}

/// Write the JSON artifact. Both the plain and the sweep path call this: a sweep
/// without an artifact is an opinion, not evidence (E3 produced exactly that
/// before this existed).
fn write_artifact(args: &Args, samples: &[Sample]) -> Result<()> {
    let Some(path) = &args.json_out else {
        return Ok(());
    };
    let doc = serde_json::json!({
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0),
        "engine": args.url,
        "fingerprint": fingerprint(&args.url),
        "model": args.model,
        "max_tokens": args.max_tokens,
        "deep_sections": args.deep_sections,
        "repeats": args.repeats,
        "samples": samples,
    });
    std::fs::write(path, serde_json::to_string_pretty(&doc)?)?;
    println!("  json   : {path}");
    Ok(())
}

/// Per shape+arm medians. The verdict is printed, not left to the reader.
fn summarize(samples: &[Sample]) {
    println!("\n  shape        arm       decode tok/s   ttft s    out tok   valid   recall");
    let mut keys: Vec<(String, String)> = Vec::new();
    for s in samples {
        let key = (s.shape.clone(), s.arm.clone());
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    for (shape, arm) in &keys {
        let group: Vec<&Sample> = samples
            .iter()
            .filter(|s| &s.shape == shape && &s.arm == arm)
            .collect();
        let decode = median(
            &group
                .iter()
                .map(|s| s.timings.decode_tps())
                .collect::<Vec<_>>(),
        );
        let ttft = median(&group.iter().map(|s| s.ttft_s).collect::<Vec<_>>());
        let chars = median(
            &group
                .iter()
                .map(|s| s.content_chars as f64)
                .collect::<Vec<_>>(),
        );
        let validity = group.iter().filter_map(|s| s.valid).filter(|v| *v).count();
        let checks = group.iter().filter(|s| s.valid.is_some()).count();
        let recalled = group
            .iter()
            .filter_map(|s| s.recall_ok)
            .filter(|v| *v)
            .count();
        let recall_checks = group.iter().filter(|s| s.recall_ok.is_some()).count();
        println!(
            "  {:<12} {:<9} {:>13.2} {:>8.2} {:>7.0} {:>7} {:>8}",
            shape,
            arm,
            decode,
            ttft,
            chars,
            if checks == 0 {
                "-".to_string()
            } else {
                format!("{validity}/{checks}")
            },
            if recall_checks == 0 {
                "-".to_string()
            } else {
                format!("{recalled}/{recall_checks}")
            }
        );
    }

    // Verdict per shape: chars (the thing we shrink) and validity (the guard).
    println!();
    for (shape, _) in keys.iter().filter(|(_, arm)| arm == "baseline") {
        let get = |arm: &str| -> Option<(f64, usize, usize)> {
            let group: Vec<&Sample> = samples
                .iter()
                .filter(|s| &s.shape == shape && s.arm == arm)
                .collect();
            if group.is_empty() {
                return None;
            }
            let chars = median(
                &group
                    .iter()
                    .map(|s| s.content_chars as f64)
                    .collect::<Vec<_>>(),
            );
            // A guard is a guard: schema validity where a schema exists, otherwise
            // the recall probe. E6's false all-arms failure taught this the hard
            // way — a run with no schema but with a recall guard is NOT unguarded.
            let schema_checks = group.iter().filter(|s| s.valid.is_some()).count();
            let recall_checks = group.iter().filter(|s| s.recall_ok.is_some()).count();
            let (checked, passed) = if schema_checks > 0 {
                (
                    schema_checks,
                    group.iter().filter(|s| s.valid == Some(true)).count(),
                )
            } else {
                (
                    recall_checks,
                    group.iter().filter(|s| s.recall_ok == Some(true)).count(),
                )
            };
            Some((chars, passed, checked))
        };
        let Some((base_chars, _, _)) = get("baseline") else {
            continue;
        };
        for arm in keys
            .iter()
            .filter(|(s, _)| s == shape)
            .map(|(_, a)| a.clone())
        {
            if arm == "baseline" {
                continue;
            }
            let Some((chars, valid, checked)) = get(&arm) else {
                continue;
            };
            let delta = if base_chars > 0.0 {
                (chars - base_chars) / base_chars * 100.0
            } else {
                0.0
            };
            let guard = if checked > 0 && valid == checked {
                "guard ok".to_string()
            } else if checked > 0 {
                format!("GUARD FAIL {valid}/{checked}")
            } else {
                "no guard".to_string()
            };
            let verdict = if delta <= -10.0 && (checked == 0 || valid == checked) {
                "ADOPT"
            } else if delta <= -3.0 {
                "marginal"
            } else {
                "null"
            };
            println!(
                "  {shape:<12} {arm:<9} tokens {:+.1}% vs baseline · {guard} · {verdict}",
                delta
            );
        }
    }
}
