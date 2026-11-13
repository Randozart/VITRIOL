use clap::Parser;

#[derive(Parser)]
#[command(name = "vitriol-calibrate")]
enum Cli {
    Calibrate {
        #[arg(long)]
        quick: bool,
        #[arg(long)]
        model: String,
    },
    /// Write a synthetic GGUF mirroring the target model's SHAPES
    /// (weights are zero-filled Q8_0 — never load real weights into it).
    Synth {
        /// The real GGUF whose header supplies the shapes.
        #[arg(long)]
        model: String,
        /// tiny (seconds-to-load) | mid (~1 GB)
        #[arg(long, default_value = "tiny")]
        scale: String,
        #[arg(long)]
        out: String,
    },
    /// Model-free hardware probe: GPUs, host RAM/zram/swap, storage
    /// bandwidth of the model store. Writes a fingerprinted report to
    /// ~/.vitriol/probes/ and prints it.
    ProbeHw {
        /// Optional file to read for storage bandwidth (the real GGUF).
        #[arg(long)]
        model: Option<String>,
    },    /// llama-bench load grid: throughput vs ubatch on a model-load-light
    /// GGUF (engine test model or synthetic). Runs llama-bench per combo,
    /// writes a fingerprinted JSON report.
    ProbeLoad {
        #[arg(long, default_value = "/home/randozart/Desktop/Projects/VITRIOL/engine/tools/server/tests/tmp/models--ggml-org--test-model-stories260K/snapshots/479896ec924af6d40fd419ab8f4d1eb2101de00d/stories260K-f32.gguf")]
        model: String,
        #[arg(long)]
        also: Vec<String>,
        #[arg(long, default_value = "64,128,256")]
        ubatch: String,
        #[arg(long, default_value = "512")]
        prompt: String,
        #[arg(long, default_value = "64")]
        gen: String,
    },
    /// Hardware + estimator synthesis: ranked config candidates with
    /// measured-vs-extrapolated confidence labels; --write-profile writes a
    /// PROFILE file (never the live config).
    ProbeRecommend {
        #[arg(long, default_value = "/home/randozart/Models/Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf")]
        model: String,
        #[arg(long)]
        write_profile: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli {
        Cli::Synth { model, scale, out } => {
            let mi = vitriol_calibrate::gguf::read_gguf(std::path::Path::new(&model))?;
            let scale = match scale.as_str() {
                "tiny" => vitriol_calibrate::synth::SynthScale::Tiny,
                "mid" => vitriol_calibrate::synth::SynthScale::Mid,
                other => anyhow::bail!("unknown scale '{other}' (tiny|mid)"),
            };
            let spec = vitriol_calibrate::synth::spec_from_model(&mi, scale);
            println!(
                "synth: arch={} layers={} embd={} heads={} kv_heads={} experts={} used={} ffn={} ctx={}",
                spec.arch, spec.layers, spec.n_embd, spec.n_head, spec.n_kv_head,
                spec.n_experts, spec.n_expert_used, spec.ffn, spec.ctx
            );
            let bytes =
                vitriol_calibrate::synth::write_synthetic(std::path::Path::new(&out), &spec)?;
            println!("wrote {} ({} MiB)", out, bytes / (1024 * 1024));
        }
        Cli::ProbeHw { model } => {
            let hw = vitriol_calibrate::probe::probe_hardware()?;
            let mem = vitriol_calibrate::probe::probe_host_memory()?;
            let mut report = serde_json::json!({
                "argv": std::env::args().collect::<Vec<_>>(),
                "gpus": hw.gpus,
                "gpu_count": hw.gpu_count,
                "host_memory": mem,
            });
            if let Some(m) = model {
                let bw =
                    vitriol_calibrate::probe::probe_storage_bandwidth(std::path::Path::new(&m))?;
                println!(
                    "  -> store read: {:.2} GiB/s ({:.2}s)",
                    bw.gib_per_s, bw.seconds
                );
                report["storage"] = serde_json::to_value(&bw)?;
            }
            for gpu in &hw.gpus {
                println!(
                    "  -> [{}] {} {} MiB (cc {})",
                    gpu.index, gpu.name, gpu.vram_mib, gpu.compute_cap
                );
            }
            println!(
                "  -> RAM {} MiB (avail {}) · swap {} MiB used of {} (zram={})",
                mem.total_mib,
                mem.available_mib,
                mem.swap_used_mib,
                mem.swap_total_mib,
                mem.zram_device
            );
            let dir = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".vitriol")
                .join("probes");
            std::fs::create_dir_all(&dir)?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let path = dir.join(format!("probe-hw-{stamp}.json"));
            std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
            println!("report: {}", path.display());
        }
        Cli::ProbeLoad { model, also, ubatch, prompt, gen } => {
            let bench = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Desktop/Projects/VITRIOL/engine/build/bin/llama-bench");
            let dir = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".vitriol")
                .join("probes");
            std::fs::create_dir_all(&dir)?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let mut report = serde_json::json!({
                "argv": std::env::args().collect::<Vec<_>>(),
                "grid": [],
            });
            let mut models = vec![model];
            models.extend(also);
            for m in &models {
                for ub in ubatch.split(',') {
                    let ub = ub.trim();
                    let out = std::process::Command::new(&bench).args([
                        "-m", m, "-p", prompt.trim(), "-n", gen.trim(),
                        "-ub", ub, "-ngl", "99", "-ts", "27,21",
                    ]).output();
                    let Ok(o) = out else { continue };
                    let text = String::from_utf8_lossy(&o.stdout);
                    let mut pp = None::<f64>;
                    let mut tg = None::<f64>;
                    for line in text.lines() {
                        // robust row parse: the test cell names pp/tg; the t/s
                        // cell is the LAST cell containing the ± marker
                        if !line.contains("±") {
                            continue;
                        }
                        let cells: Vec<&str> =
                            line.split('|').map(|c| c.trim()).collect();
                        let ts_cell = cells
                            .iter()
                            .filter(|c| c.contains("±"))
                            .next_back()
                            .copied()
                            .unwrap_or("");
                        let v = ts_cell
                            .split_whitespace()
                            .next()
                            .and_then(|x| x.parse::<f64>().ok());
                        if cells.iter().any(|c| c.starts_with(&format!("pp{}", prompt.trim()))) {
                            pp = v;
                        } else if cells.iter().any(|c| c.starts_with(&format!("tg{}", gen.trim()))) {
                            tg = v;
                        }
                    }
                    report["grid"].as_array_mut().unwrap().push(serde_json::json!({
                        "model": std::path::Path::new(m)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        "ubatch": ub,
                        "pp": pp,
                        "tg": tg,
                    }));
                    println!(
                        "{} ub={}: pp={:?} tg={:?}",
                        std::path::Path::new(m).file_name().unwrap_or_default().to_string_lossy(),
                        ub, pp, tg
                    );
                }
            }
            let path = dir.join(format!("probe-load-{stamp}.json"));
            std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
            println!("report: {}", path.display());
        }

        Cli::ProbeRecommend { model, write_profile } => {
            let hw = vitriol_calibrate::probe::probe_hardware()?;
            let mi = vitriol_calibrate::gguf::read_gguf(std::path::Path::new(&model))?;
            let (est, opt) = vitriol_calibrate::estimator::estimate_vram(&mi, &hw);

            // KV per token DERIVED from the model's own tensor catalog:
            // attention-layer count (blk.*.attn_k.weight occurrences) × K/V
            // width (attn_k ne[1]) × 2 × q4_0 bytes/element (0.5625 B).
            // The estimator's kvt undercounts hybrid models (per-layer only,
            // raw head dim) — both values are printed for honesty.
            let attn_layers = mi
                .tensors
                .iter()
                .filter(|t| t.name.ends_with("attn_k.weight"))
                .count()
                .max(1);
            let kv_width = mi
                .tensors
                .iter()
                .find(|t| t.name.ends_with("attn_k.weight"))
                .and_then(|t| t.shape.get(1).copied())
                .unwrap_or(0)
                .max(1) as f64;
            let kv_kib_per_token = attn_layers as f64 * kv_width * 2.0 * 0.5625 / 1024.0;
            println!("recommend: {} ({} layers, {} experts)", mi.architecture, mi.block_count, mi.expert_count);
            println!("  weights        {:.1} GiB", mi.total_size_bytes as f64 / (1u64 << 30) as f64);
            println!(
                "  KV/token       {:.2} KiB (derived: {attn_layers} attention layers x {kv_width:.0} kv-width, q4_0; estimator says {:.2} KiB — per-layer only)",
                kv_kib_per_token,
                est.kv_mib_per_token * 1024.0
            );
            println!("  usable VRAM    {:.0} MiB (90% of {:.0})", est.vram_usable_mib, est.vram_total_mib);
            println!();
            println!("  ctx ladder (extrapolated from the estimator; KV = ctx x KiB/token):");
            for ctx in [131_072u64, 208_000, 262_144] {
                let kv_mib = kv_kib_per_token * ctx as f64 / 1024.0;
                let total = est.base_model_mib + kv_mib;
                let pct = total / est.vram_usable_mib * 100.0;
                let verdict = if total <= est.vram_usable_mib { "fits" } else { "OVER" };
                println!("    ctx {ctx:>7}: kv {kv_mib:>7.1} MiB · total {total:>7.1} MiB ({pct:>5.1}%) {verdict}");
            }
            println!();
            println!("  recommended (estimator OptimalConfig):");
            println!("    tensor_split     {}", opt.tensor_split);
            println!("    ctx              {}", opt.context);
            println!("    ubatch           {}", opt.ubatch_size);
            println!("    kv quant         k={} v={}", opt.k_quant, opt.v_quant);
            println!("    draft_n_max      {}", opt.draft_n_max);
            println!("    pin_first_n      {}", opt.pin_first_n_layers);
            println!("    est VRAM         {:.1} MiB ({:.1}%)", opt.estimated_vram_mib, opt.estimated_vram_pct);
            println!();

            // measured grid, if a probe-load report exists
            let probes = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".vitriol").join("probes");
            let mut newest: Option<std::path::PathBuf> = None;
            if let Ok(rd) = std::fs::read_dir(&probes) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.file_name().map(|n| n.to_string_lossy().starts_with("probe-load-")).unwrap_or(false) {
                        let newer = newest.as_ref().map(|n| {
                            std::fs::metadata(&p).and_then(|m| m.modified()).ok()
                                > std::fs::metadata(n).and_then(|m| m.modified()).ok()
                        }).unwrap_or(true);
                        if newer { newest = Some(p); }
                    }
                }
            }
            if let Some(p) = newest {
                if let Ok(txt) = std::fs::read_to_string(&p) {
                    if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&txt) {
                        println!("  measured grid ({}):", p.file_name().unwrap_or_default().to_string_lossy());
                        if let Some(rows) = doc["grid"].as_array() {
                            for r in rows {
                                println!(
                                    "    {} ub={} → pp {} t/s · tg {} t/s",
                                    r["model"].as_str().unwrap_or("?"),
                                    r["ubatch"].as_str().unwrap_or("?"),
                                    r["pp"].as_f64().map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into()),
                                    r["tg"].as_f64().map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into())
                                );
                            }
                        }
                        println!("    (measured on load-light test models — ubatch/split/KV curves transfer; absolute t/s do not)");
                    }
                }
            } else {
                println!("  no probe-load report yet — run: vitriol probe load");
            }
            println!();
            println!("  codex: docs/engine-settings.md (E-gpu.tensor_split, E-kv.ubatch_size, E-kv.quant, E-spec.draft_n_max)");

            if let Some(name) = write_profile {
                let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
                let live = home.join(".vitriol").join("config");
                let dir = home.join(".vitriol").join("profiles").join(&name);
                if dir.exists() {
                    anyhow::bail!("profile '{name}' already exists — pick another name or remove it first");
                }
                std::fs::create_dir_all(&dir)?;
                let text = std::fs::read_to_string(&live)?;
                let text = set_ini(&text, "gpu", "tensor_split", &opt.tensor_split);
                let text = set_ini(&text, "model", "context", &opt.context.to_string());
                let text = set_ini(&text, "kv", "ubatch_size", &opt.ubatch_size.to_string());
                let text = set_ini(&text, "kv", "quant_mode", &opt.k_quant);
                let text = set_ini(&text, "kv", "quant_mode_v", &opt.v_quant);
                let text = set_ini(&text, "spec", "draft_n_max", &opt.draft_n_max.to_string());
                std::fs::write(dir.join("config"), text)?;
                std::fs::write(
                    dir.join("meta"),
                    format!(
                        "name={name}\ndescription=probe-recommend ({:.1}% est VRAM; estimator + probe-load grid)\ncreated={}\n",
                        opt.estimated_vram_pct,
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
                    ),
                )?;
                println!();
                println!("  profile written: {} (load it explicitly with: vitriol config load {name})", dir.display());
            }
        }

        Cli::Calibrate { quick: true, model } => {
            let path = std::path::Path::new(&model);

            println!("Probing hardware...");
            let hw = vitriol_calibrate::probe::probe_hardware()?;
            for gpu in &hw.gpus {
                println!(
                    "  -> [{}] {} {} MiB (cc {}, pcie gen {} x{})",
                    gpu.index,
                    gpu.name,
                    gpu.vram_mib,
                    gpu.compute_cap,
                    gpu.pcie_gen,
                    gpu.pcie_width
                );
            }
            println!("  -> {} GPU(s) total", hw.gpu_count);

            println!("Analyzing model...");
            let mi = vitriol_calibrate::gguf::read_gguf(path)?;
            println!(
                "  -> {}, {} layers, {} experts, embd={}, n_head={}, n_kv_head={}",
                mi.architecture,
                mi.block_count,
                mi.expert_count,
                mi.embedding_length,
                mi.head_count,
                mi.head_count_kv
            );

            println!("Estimating VRAM bounds...");
            let (est, opt) = vitriol_calibrate::estimator::estimate_vram(&mi, &hw);

            println!();
            println!(
                "Optimal: pin{} ctx{} ubatch{} MTP{}",
                opt.pin_first_n_layers, opt.context, opt.ubatch_size, opt.draft_n_max
            );
            println!(
                "  Estimated VRAM: {:.0} MiB ({:.1}%)",
                opt.estimated_vram_mib, opt.estimated_vram_pct
            );
            println!();
            println!("Derivation:");
            println!("  Base model: {:.0} MiB", est.base_model_mib);
            println!(
                "  Pinned experts ({} x {:.0}): {:.0} MiB",
                opt.pin_first_n_layers,
                est.per_layer_pin_cost_mib,
                opt.pin_first_n_layers as f64 * est.per_layer_pin_cost_mib
            );
            println!(
                "  KV cache: {:.1} MiB",
                opt.context as f64 * est.kv_mib_per_token
            );
            println!("  CUDA overhead: {:.0} MiB", est.overhead_mib);
            println!(
                "  Usable: {:.0} MiB ({:.0}%)",
                est.vram_usable_mib,
                est.vram_safety_margin * 100.0
            );
            if !opt.tensor_split.is_empty() {
                println!(
                    "Split hint (compute-aware): llama-server -sm layer --tensor-split {} --main-gpu <fastest>",
                    opt.tensor_split
                );
            }
        }
        _ => {
            eprintln!("Usage: vitriol-calibrate calibrate --quick --model PATH");
            std::process::exit(1);
        }
    }
    Ok(())
}


/// Replace `key = value` inside `[section]` (config/profile INI text).
/// Unknown sections/keys are left untouched (the profile starts from the
/// live config, so every key already exists).
fn set_ini(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current = trimmed[1..trimmed.len() - 1].trim().to_string();
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let matches_key = current == section
            && trimmed
                .split('=')
                .next()
                .map(|k| k.trim() == key)
                .unwrap_or(false);
        if matches_key {
            out.push_str(&format!("{key} = {value}"));
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
