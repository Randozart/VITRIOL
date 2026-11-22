use crate::gguf::ModelInfo;
use crate::probe::HardwareInfo;

pub struct VramEstimate {
    pub base_model_mib: f64,
    pub per_layer_pin_cost_mib: f64,
    pub kv_mib_per_token: f64,
    pub overhead_mib: f64,
    pub vram_total_mib: f64,
    pub vram_safety_margin: f64,
    pub vram_usable_mib: f64,
}

pub struct OptimalConfig {
    pub pin_first_n_layers: u32,
    pub context: u32,
    pub ubatch_size: u32,
    pub draft_n_max: u32,
    pub k_quant: String,
    pub v_quant: String,
    pub tensor_split: String,
    pub estimated_vram_mib: f64,
    pub estimated_vram_pct: f64,
}

fn gpu_overhead(cc: &str) -> f64 {
    if cc.starts_with("6.") {
        1800.0
    } else if cc.starts_with("7.") {
        2200.0
    } else if cc.starts_with("8.") {
        2800.0
    } else if cc.starts_with("9.") {
        3200.0
    } else {
        2000.0
    }
}

/* Compute-aware relative per-device throughput heuristic for --tensor-split.
 * Pure VRAM balance is a poor split when the GPU pair has wildly different
 * per-layer latency (Pascal: no FlashAttention + 1/64 fp16; Ampere: FA + fast
 * fp16). These weights bias the llama.cpp split towards the fast card. The
 * real autotune (measured per-card layer timing) lands once the second GPU is
 * present. */
fn gpu_perf_weight(cc: &str) -> u32 {
    if cc.starts_with("6.") {
        10
    } else if cc.starts_with("7.") {
        16
    } else if cc.starts_with("8.") {
        26
    } else if cc.starts_with("9.") {
        36
    } else {
        10
    }
}

/* Per-architecture KV correction factor, applied to the generic
 * kv_mib_per_token heuristic. Default 1.0 keeps every existing arch's
 * estimate byte-identical (invariant I9). Only looped hybrid-attention
 * archs (Inkling-2) get a correction: their generic per-token KV formula
 * assumes every layer is global attention, but a fraction of layers are
 * sliding-window / recurrent and store O(1) state rather than growing
 * with context, so the effective per-context KV is lower.
 * The factor is a documented first-order estimate (pending a measured KV
 * bytes/token from a successful run); it is arch-gated so no other
 * model is affected. */
fn arch_kv_factor(arch: &str) -> f64 {
    if arch.starts_with("inkling2") {
        0.5
    } else {
        1.0
    }
}

pub fn estimate_vram(model: &ModelInfo, hw: &HardwareInfo) -> (VramEstimate, OptimalConfig) {
    /* Unified VRAM pool: sum across every device. For a single-GPU system
     * this reduces to the old per-device math, so nothing regresses. */
    let vram_total: f64 = hw.gpus.iter().map(|g| g.vram_mib as f64).sum();
    let usable = vram_total * 0.9;
    let bc = model.block_count.max(1) as f64;

    let base_mib =
        (model.total_size_bytes as f64 - model.per_layer_experts_bytes as f64 * bc) / 1_048_576.0;
    let layer_mib = model.per_layer_experts_bytes as f64 / 1_048_576.0;

    let hd = model.embedding_length as f64 / model.head_count.max(1) as f64;
    let kvf = arch_kv_factor(&model.architecture);
    let kvt = hd * model.head_count_kv.max(1) as f64 * 2.5 / 1_048_576.0 * kvf;

    /* CUDA context overhead is per-device — each GPU pays its own. */
    let oh: f64 = if hw.gpus.is_empty() {
        2000.0
    } else {
        hw.gpus.iter().map(|g| gpu_overhead(&g.compute_cap)).sum()
    };
    let max_block = model.block_count.min(24) as u32;

    let mut best_score = i64::MIN;
    let mut bp = 0u32;
    let mut bctx = 65536u32;
    let mut bub = 128u32;
    let mut bv = 0.0;

    for &ctx in &[131072u32, 65536, 32768] {
        for &ub in &[128u32, 256] {
            let scr = ub as f64 * model.embedding_length as f64 * 4.0 / 1_048_576.0;
            let mut mf = 0u32;
            for p in (0..=max_block).step_by(2) {
                let v = base_mib + p as f64 * layer_mib + ctx as f64 * kvt + scr + oh;
                if v <= usable {
                    mf = p;
                }
            }
            if mf == 0 {
                continue;
            }
            let v = base_mib + mf as f64 * layer_mib + ctx as f64 * kvt + scr + oh;
            let s = (mf as i64) * 1_000_000 + (ctx as i64) * 1000 - (ub as i64);
            if s > best_score {
                best_score = s;
                bp = mf;
                bctx = ctx;
                bub = ub;
                bv = v;
            }
        }
    }
    if bp == 0 && bv == 0.0 {
        bv = base_mib + 8192.0 * kvt + 1.0 + oh;
        bctx = 8192;
    }
    let draft = (model.block_count / 8).clamp(1, 5) as u32;

    /* Compute-aware --tensor-split hint (only meaningful with 2+ GPUs). */
    let tensor_split: String = if hw.gpus.len() >= 2 {
        hw.gpus
            .iter()
            .map(|g| gpu_perf_weight(&g.compute_cap).to_string())
            .collect::<Vec<_>>()
            .join(",")
    } else {
        String::new()
    };

    (
        VramEstimate {
            base_model_mib: base_mib,
            per_layer_pin_cost_mib: layer_mib,
            kv_mib_per_token: kvt,
            overhead_mib: oh,
            vram_total_mib: vram_total,
            vram_safety_margin: 0.9,
            vram_usable_mib: (usable * 10.0).round() / 10.0,
        },
        OptimalConfig {
            pin_first_n_layers: bp,
            context: bctx,
            ubatch_size: bub,
            draft_n_max: draft,
            k_quant: "q4_0".into(),
            v_quant: "f16".into(),
            tensor_split,
            estimated_vram_mib: (bv * 10.0f64).round() / 10.0,
            estimated_vram_pct: ((bv / vram_total * 100.0f64) * 10.0).round() / 10.0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gguf::ModelInfo;
    use crate::probe::{GpuInfo, HardwareInfo};

    /* Build a model with the arch swapped; every other field identical so
     * the only variable is the arch-gated KV correction. */
    fn model(arch: &str) -> ModelInfo {
        ModelInfo {
            architecture: arch.to_string(),
            context_length: 0,
            block_count: 24,
            expert_count: 128,
            expert_used_count: 8,
            embedding_length: 5120,
            head_count: 64,
            head_count_kv: 8,
            has_mtp: false,
            total_size_bytes: 12_000_000_000,
            tensor_count: 704,
            per_layer_attn_bytes: 100_000,
            per_layer_experts_bytes: 200_000,
            tensors: Vec::new(),
        }
    }

    fn hw() -> HardwareInfo {
        HardwareInfo {
            probed_at: String::new(),
            gpus: vec![GpuInfo {
                index: 0,
                name: "GPU".to_string(),
                vram_mib: 12_000,
                compute_cap: "8.6".to_string(),
                pcie_gen: 4,
                pcie_width: 16,
            }],
            cpu: String::new(),
            has_avx2: true,
            ram_mib: 32_000,
            gpu_count: 1,
            has_ipc_lock: true,
        }
    }

    #[test]
    fn arch_factor_defaults_to_one() {
        assert_eq!(arch_kv_factor("qwen3"), 1.0);
        assert_eq!(arch_kv_factor("llama"), 1.0);
        assert_eq!(arch_kv_factor("gpt2"), 1.0);
    }

    #[test]
    fn arch_factor_firest_on_inkling2() {
        assert_eq!(arch_kv_factor("inkling2"), 0.5);
        assert_eq!(arch_kv_factor("inkling2_loop"), 0.5);
    }

    /* Invariant I9: a non-inkling arch must be byte-identical to the generic
     * formula (kvf = 1.0), i.e. the correction touches nothing else. */
    #[test]
    fn non_inkling_arch_is_unaffected() {
        let hw = hw();
        let (est, _cfg) = estimate_vram(&model("qwen3"), &hw);
        let hd = 5120.0 / 64.0;
        let expected_generic = hd * 8.0 * 2.5 / 1_048_576.0; // kvf = 1.0
        assert!(
            (est.kv_mib_per_token - expected_generic).abs() < 1e-12,
            "I9: 27B-class arch must keep the generic KV estimate, got {} want {}",
            est.kv_mib_per_token,
            expected_generic
        );
    }

    /* The inkling2 correction fires: its per-token KV is exactly half the
     * generic value for identical geometry. */
    #[test]
    fn inkling2_arch_applies_hybrid_kv_correction() {
        let hw = hw();
        let (est, _cfg) = estimate_vram(&model("inkling2_loop"), &hw);
        let hd = 5120.0 / 64.0;
        let expected = hd * 8.0 * 2.5 / 1_048_576.0 * 0.5;
        assert!(
            (est.kv_mib_per_token - expected).abs() < 1e-12,
            "inkling2 KV correction should halve the generic estimate, got {} want {}",
            est.kv_mib_per_token,
            expected
        );
    }
}
