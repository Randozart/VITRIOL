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

pub fn estimate_vram(model: &ModelInfo, hw: &HardwareInfo) -> (VramEstimate, OptimalConfig) {
    let vram_total = hw.gpus.first().map(|g| g.vram_mib as f64).unwrap_or(8192.0);
    let usable = vram_total * 0.9;
    let bc = model.block_count.max(1) as f64;

    let base_mib =
        (model.total_size_bytes as f64 - model.per_layer_experts_bytes as f64 * bc) / 1_048_576.0;
    let layer_mib = model.per_layer_experts_bytes as f64 / 1_048_576.0;

    let hd = model.embedding_length as f64 / model.head_count.max(1) as f64;
    let kvt = hd * model.head_count_kv.max(1) as f64 * 2.5 / 1_048_576.0;

    let oh = hw
        .gpus
        .first()
        .map(|g| gpu_overhead(&g.compute_cap))
        .unwrap_or(2000.0);
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
            estimated_vram_mib: (bv * 10.0f64).round() / 10.0,
            estimated_vram_pct: ((bv / vram_total * 100.0f64) * 10.0).round() / 10.0,
        },
    )
}
