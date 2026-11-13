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
