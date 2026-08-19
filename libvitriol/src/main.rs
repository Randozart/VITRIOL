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
}

fn main() -> anyhow::Result<()> {
    let cli: Cli = Cli::parse();
    match cli {
        Cli::Calibrate { quick: true, model } => {
            let path = std::path::Path::new(&model);

            println!("Probing hardware...");
            let hw = vitriol_calibrate::probe::probe_hardware()?;
            for gpu in &hw.gpus {
                println!(
                    "  -> [{}] {} {} MiB (cc {}, pcie gen {} x{})",
                    gpu.index, gpu.name, gpu.vram_mib, gpu.compute_cap,
                    gpu.pcie_gen, gpu.pcie_width
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
