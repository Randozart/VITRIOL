//! Vitriol-tab Config view (settings-codex/probe stage 6, 2026-11-13).
//!
//! Parses the LIVE `~/.vitriol/config` (flat `key = value` under
//! `[sections]`) and renders each key with a one-line explanation from the
//! static dictionary below — the information lives where the operator would
//! change things. READ-ONLY: this view never writes the config (I6).
//!
//! Dictionary coverage is enforced by a test against
//! `tests/fixtures/vitriol-config-keys.txt` (the full live key list).
//! Entries mirror `docs/engine-settings.md`; keep them in step.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRow {
    pub section: String,
    pub key: String,
    pub value: String,
    pub explain: &'static str,
    /// Risk/forbidden flags render amber in the UI.
    pub risk: bool,
}

/// section.key → (one-line explanation, risk).
pub const DICTIONARY: &[(&str, &str, bool)] = &[
    (
        "gpu.device",
        "CUDA device index for the primary GPU (logits/sampling)",
        false,
    ),
    (
        "gpu.exclude_secondary",
        "exclude the second GPU entirely (was for the old CC 5.2 card)",
        false,
    ),
    (
        "gpu.tensor_split",
        "layer split across GPUs — 27,21 is the 2026-09-28 blessed point (lares off)",
        false,
    ),
    (
        "gpu.rpc_servers",
        "remote RPC layer-split targets (needs -DGGML_RPC=ON + matching 3-way split)",
        false,
    ),
    (
        "model.path",
        "the daily-driver GGUF (BuffedMod IQ3_S_XL + Q6K output)",
        false,
    ),
    (
        "model.context",
        "native 262K window; depth, not window, is the real wall",
        false,
    ),
    (
        "model.threads",
        "CPU prompt threads (2/4/8 is a cheap probe-load candidate)",
        false,
    ),
    (
        "model.ngl",
        "layers offloaded — 99 = all (partial offload pours experts over PCIe)",
        false,
    ),
    (
        "model.expert_count",
        "expert override; 0 = model default (never truncate this family)",
        false,
    ),
    (
        "model.alias",
        "LOAD-BEARING: /v1/models identity pi model-sync matches on",
        true,
    ),
    (
        "model.mmproj",
        "LOAD-BEARING: vision projector; non-empty DISABLES slot save/restore",
        true,
    ),
    (
        "model.n_cpu_moe",
        "CPU-MoE for first N layers; off while both GPUs are resident",
        false,
    ),
    (
        "model.mmap",
        "off = weights hot-loaded in VRAM (they fit; avoids eviction stalls)",
        false,
    ),
    (
        "vitriol.mode",
        "streaming/DMA mode; off = fully VRAM-resident (residency rule)",
        false,
    ),
    (
        "vitriol.disk_offload",
        "DMA expert streaming from disk (needs mode != off)",
        false,
    ),
    (
        "vitriol.store",
        "raw-zone expert store (needs mode=stream + max_locked_mb)",
        false,
    ),
    (
        "vitriol.store_path",
        "store device/file path (empty = disabled)",
        false,
    ),
    (
        "vitriol.store_source",
        "store fill source: store | gguf",
        false,
    ),
    (
        "vitriol.max_locked_mb",
        "lazy per-expert host pin budget (0 = pin all; needs RAM >= experts)",
        false,
    ),
    (
        "vitriol.lru_mb_gpu",
        "per-device LRU pool override CSV (empty = global lru_mb)",
        false,
    ),
    (
        "vitriol.lru_mb",
        "LRU VRAM cache for FP16/FP32 experts (dormant when mode=off)",
        false,
    ),
    (
        "vitriol.verbose",
        "verbose engine logging (off keeps the journal readable)",
        false,
    ),
    (
        "vitriol.output_cache",
        "approximate expert-output reuse (quality risk; pin begins auto-disables)",
        false,
    ),
    (
        "vitriol.predictive_prefetch",
        "cross-layer expert prefetch (dormant when mode=off)",
        false,
    ),
    (
        "vitriol.pin_first_n_layers",
        "pin first N layers' experts (~220 MB/layer; unproven on BuffedMod — E4)",
        false,
    ),
    (
        "vitriol.prune_experts",
        "drop bottom N of 8 active experts (experimental quality risk)",
        true,
    ),
    (
        "vitriol.reasoning",
        "thinking-mode disable (required for the Qwen3.6 lineage)",
        false,
    ),
    (
        "server.host",
        "API bind — loopback only (no auth layer)",
        false,
    ),
    (
        "server.port",
        "API port 8279 — the TUI/opencode/Hermes all target it",
        true,
    ),
    (
        "server.parallel",
        "slots; 1 per the single-session doctrine (concurrency rides overflow lanes)",
        false,
    ),
    (
        "server.slot_context",
        "per-slot context override (empty = model.context)",
        false,
    ),
    (
        "server.slot_save_path",
        "/scratch/kvtier — XFS NVMe KV mirror, warmed by the boot sidecar",
        false,
    ),
    (
        "server.cache_ram",
        "host-RAM prompt cache MiB; 0 is FORBIDDEN (no readiness)",
        true,
    ),
    (
        "server.cache_idle_slots",
        "idle-slot KV to host RAM (off since the 09-04 host-memory trim)",
        false,
    ),
    (
        "server.cache_reuse",
        "KV-shift salvage chunk (0 = off; E3 protocol)",
        false,
    ),
    (
        "server.ctx_resize",
        "live KV-window swap endpoint (governor RESIZE rung); fingerprint cxr=",
        false,
    ),
    (
        "server.chat_template",
        "custom Jinja template (load-bearing: changes every prompt's serialization)",
        true,
    ),
    ("memory.mode", "emulated-memory mode (off)", false),
    (
        "memory.semantic_mode",
        "semantic search over memories (needs sentence-transformers)",
        false,
    ),
    (
        "kv.mode",
        "sparse = probe-scored eviction (the daily driver's depth enabler)",
        false,
    ),
    (
        "kv.score",
        "probe scoring feeds eviction (off = blind)",
        false,
    ),
    (
        "kv.score_every",
        "probe cadence in decode steps; 4 was REJECTED (ejected too hard)",
        false,
    ),
    (
        "kv.score_min_fill",
        "score only when cache fill >= this (0.5 removes the shallow-turn penalty)",
        false,
    ),
    (
        "kv.quant_mode",
        "K cache quant (q4_0 = 09-28 blessed; tq3_0 for headroom)",
        false,
    ),
    (
        "kv.quant_mode_v",
        "V cache quant — q4_0 passed here but V-quant caused garbage on other families",
        true,
    ),
    (
        "kv.frozen_prompt",
        "static system prompt (fights the preservation tails; off)",
        false,
    ),
    (
        "kv.ubatch_size",
        "physical batch; lower = smaller compute buffers — E1 drift candidate (128 vs 64)",
        false,
    ),
    (
        "kv.checkpoint_every_n_tokens",
        "slot checkpoint cadence (8192 = IO vs crash-loss balance)",
        false,
    ),
    (
        "kv.ctx_checkpoints",
        "RAM checkpoint ring per slot; 0 is FORBIDDEN (heap corruption)",
        true,
    ),
    (
        "engine.mode",
        "execution mode; vitriol-dma is the fork's DMA-capable path",
        false,
    ),
    (
        "lookup.tokens",
        "prompt-lookup decoding (off — MTP is the blessed draft source)",
        false,
    ),
    (
        "spec.type",
        "speculative decoding type (mtp = embedded MTP head)",
        false,
    ),
    (
        "spec.draft_n_max",
        "draft tokens; n=2 is live (11-11 A/B: 0.953 vs 0.735 at n=3)",
        false,
    ),
    (
        "spec.adaptive_k",
        "acceptance-driven draft trim (never A/B'd on BuffedMod — E2)",
        false,
    ),
    (
        "spec.draft_model",
        "separate draft model (empty = embedded MTP head)",
        false,
    ),
    (
        "spec.draft_device",
        "draft device override (empty = main devices; dev1 side has the headroom)",
        false,
    ),
    (
        "spec.draft_ngl",
        "draft layers in VRAM (all = embedded head, correct)",
        false,
    ),
    (
        "spec.draft_type_k",
        "draft K quant (empty = f16; q4_0 draft KV breaks flash-attn -> load OOM)",
        true,
    ),
    (
        "spec.draft_type_v",
        "draft V quant (empty = f16; same OOM warning)",
        true,
    ),
    (
        "chimera.mode",
        "backend routing auto|cuda|vulkan|off (both collapse to CUDA with mode=off)",
        false,
    ),
];

impl ConfigRow {
    pub fn label(&self) -> String {
        format!("{}.{}", self.section, self.key)
    }
}

pub fn explain(section: &str, key: &str) -> (&'static str, bool) {
    let want = format!("{section}.{key}");
    DICTIONARY
        .iter()
        .find(|(k, _, _)| *k == want)
        .map(|(_, e, r)| (*e, *r))
        .unwrap_or((
            "(no explanation yet — add it to docs/engine-settings.md)",
            false,
        ))
}

/// Parse INI text into rows (comments/blank lines skipped; unknown keys get
/// the honest fallback explanation).
pub fn parse_rows(text: &str) -> Vec<ConfigRow> {
    let mut section = String::new();
    let mut rows = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_string();
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim().to_string();
        if key.is_empty() || section.is_empty() {
            continue;
        }
        let value = v.trim().to_string();
        let (explain, risk) = explain(&section, &key);
        rows.push(ConfigRow {
            section: section.clone(),
            key,
            value,
            explain,
            risk,
        });
    }
    rows
}

pub fn config_path() -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("config");
    p
}

pub fn load() -> Option<Vec<ConfigRow>> {
    let text = std::fs::read_to_string(config_path()).ok()?;
    Some(parse_rows(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_covers_every_live_key() {
        // The committed fixture is the full live key list (no values).
        let keys = include_str!("../../tests/fixtures/vitriol-config-keys.txt");
        let mut missing = Vec::new();
        for line in keys.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let hit = DICTIONARY.iter().any(|(k, _, _)| *k == line);
            if !hit {
                missing.push(line.to_string());
            }
        }
        assert!(
            missing.is_empty(),
            "keys missing from the Config-view dictionary (add to DICTIONARY + docs/engine-settings.md): {missing:?}"
        );
    }

    #[test]
    fn parse_skips_comments_and_keeps_sections() {
        let text = "\
# header
[gpu]
device = 0
tensor_split = 27,21

[model]
context = 262144
unknown_key = x
";
        let rows = parse_rows(text);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].label(), "gpu.device");
        assert_eq!(rows[0].value, "0");
        assert!(rows[0].explain.contains("CUDA"));
        assert_eq!(rows[3].label(), "model.unknown_key");
        assert!(rows[3].explain.contains("no explanation"));
        // risk flags surface for load-bearing keys
        let mmproj = rows.iter().find(|r| r.key == "mmproj");
        assert!(mmproj.is_none(), "fixture config has no mmproj line");
        let (_, risk) = explain("model", "mmproj");
        assert!(risk);
    }

    #[test]
    fn live_config_also_covers_when_present() {
        let Some(rows) = load() else { return };
        let unknown: Vec<String> = rows
            .iter()
            .filter(|r| r.explain.contains("no explanation"))
            .map(|r| r.label())
            .collect();
        assert!(
            unknown.is_empty(),
            "live config keys without explanation: {unknown:?}"
        );
    }
}
