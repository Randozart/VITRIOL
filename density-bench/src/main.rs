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

mod engine;
mod sample;
mod shapes;
mod sweep;

use anyhow::Result;
use engine::Client;
use sample::{median, Sample};
use shapes::Shape;

struct Args {
    url: String,
    model: String,
    arms: Vec<String>,
    shape_filter: Vec<String>,
    repeats: usize,
    max_tokens: u32,
    deep_sections: usize,
    json_out: Option<String>,
    /// Sweep mode inputs (engine binary, base args, dimensions).
    engine: Option<String>,
    base_args: String,
    sets: Vec<String>,
    env_sets: Vec<String>,
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
                       baseline = plain request, grammar = request + json_schema
  --shapes LIST        comma-separated subset of structured,code-edit,deep-recall
  --repeats N          samples per arm per shape (default 3)
  --max-tokens N       output cap per request (default 256)
  --deep-sections N    size of the deep-recall prompt (default 40)
  --json PATH          write the raw samples as JSON
  --no-stream          skip the TTFT pass (tokens only; use on a shared engine)
"
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        url: "http://127.0.0.1:8279".into(),
        model: "unknown".into(),
        arms: Vec::new(),
        shape_filter: Vec::new(),
        repeats: 3,
        max_tokens: 256,
        deep_sections: 40,
        json_out: None,
        no_stream: false,
        engine: None,
        base_args: String::new(),
        sets: Vec::new(),
        env_sets: Vec::new(),
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
                args.arms.push(value(i)?);
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
            "--env" => {
                args.env_sets.push(value(i)?);
                i += 2;
            }
            "--dry-run" => {
                args.dry_run = true;
                i += 1;
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
        args.arms.push("baseline".into());
    }
    if args.repeats == 0 {
        anyhow::bail!("--repeats must be at least 1");
    }
    Ok(args)
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
    let arms = sweep::matrix(&args.sets, &args.env_sets);
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
            stream: !args.no_stream,
        };
        for shape in shapes {
            for run in 0..args.repeats {
                match client.run(shape, &arm.label()) {
                    Ok(s) => println!(
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
                    ),
                    Err(e) => eprintln!("  {:<12} run {} failed: {e:#}", shape.id, run + 1),
                }
            }
        }
        print_eviction_metrics(&args.url);
        let _ = child.kill();
        let _ = child.wait();
    }
    println!("\n  sweep complete — logs under /tmp/density-sweep-arm*.log");
    Ok(())
}

/// Print the arm's eviction counters.
///
/// E6's whole question is what eviction DID, not only how fast the answer came
/// back: a sweep that never ejected anything measured nothing. Failures print a
/// note rather than aborting the arm.
fn print_eviction_metrics(url: &str) {
    let endpoint = format!("{}/metrics", url.trim_end_matches('/'));
    let Ok(res) = ureq::get(&endpoint).timeout(std::time::Duration::from_secs(5)).call() else {
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
        args.arms.join(" + "),
        args.repeats,
        args.max_tokens,
        args.deep_sections
    );

    let mut samples: Vec<Sample> = Vec::new();
    for shape in &shapes {
        // Paired measurement: the baseline is re-measured next to every arm, in
        // the same instant, because drift on this box is larger than most effects.
        let mut arms = args.arms.clone();
        if !arms.iter().any(|a| a == "baseline") {
            arms.insert(0, "baseline".into());
        }
        println!("  shape  : {} ({})", shape.id, shape.note);
        for arm in &arms {
            for run in 0..args.repeats {
                match client.run(shape, arm) {
                    Ok(s) => {
                        println!(
                            "  {:<12} {:<9} run {} · prompt {:>7.1} tok/s · decode {:>6.2} tok/s · \
                             ttft {:>6.2}s · {} chars{}{}",
                            shape.id,
                            arm,
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
                            }
                        );
                        samples.push(s);
                    }
                    Err(e) => {
                        eprintln!("  {:<12} {:<9} run {} failed: {e:#}", shape.id, arm, run + 1)
                    }
                }
            }
        }
    }

    summarize(&samples);
    if let Some(path) = args.json_out {
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
        std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
        println!("  json   : {path}");
    }
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
            let checked = group.iter().filter(|s| s.valid.is_some()).count();
            let valid = group.iter().filter(|s| s.valid == Some(true)).count();
            Some((chars, valid, checked))
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
