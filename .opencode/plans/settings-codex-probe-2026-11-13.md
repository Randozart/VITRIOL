# Settings codex + vitriol probe subsystem

- Date: 2026-11-13
- Status: implemented 2026-11-13 — stages 1–6 landed.
  - Stage 1 (d3ceb3c): docs/engine-settings.md, full live-key coverage verified.
  - Stage 2 (e6fe616): synth generator + hardware/storage probes. Synth
    llama-bench loadability: TODO (header parses; loader-side tensor-count
    mismatch remains; probe load uses the engine's tiny test models).
    Bugs caught by tests along the way: missing kv_count, wrong GGML type
    ids, attention.* key names, Q8_0 block alignment, infinite pad loop,
    order-dependent metadata extraction.
  - Stage 3 (d6bef34): probe load grid via llama-bench (measured numbers).
  - Stage 4 (ef3d05d): probe recommend with KV derived from the tensor
    catalog — 19.12 KiB/token, matching the AGENTS certification exactly;
    opt-in profile write.
  - Stage 5 (de6b9ab): drift protocols E1–E4 in the codex.
  - Stage 6: TUI Config view in the Vitriol tab (read-only, dictionary
    coverage test over tests/fixtures/vitriol-config-keys.txt), rebuilt +
    installed.
- Origin: operator observation — the daily driver works but the settings
  are opaque and there is no information anywhere to judge better values.
  Decisions: everything lands in default VITRIOL; probe tools are
  model-load-light; two synthetic scales; recommendations print + opt-in
  `--write-profile`; codex doc first.

## Goal

1. A **settings codex** (`docs/engine-settings.md`): every live config key
   explained, with the recorded evidence for its current value.
2. A **probe subsystem** (`vitriol probe hardware|load|recommend`):
   hardware + load-characteristic probing that never loads real model
   weights — synthetic GGUFs mirror the real model's shapes, llama-bench
   measures this hardware at those shapes across settings, and the
   recommender combines probes + the VRAM estimator into ranked,
   confidence-labelled candidate configs.
3. A **drift audit**: live config vs blessed values, each delta with a
   test protocol.
4. A **TUI Config view**: the live config with one-line explanations in
   the Vitriol tab, dictionary coverage enforced by test.

## Stages

1. `docs/engine-settings.md` — per-key entries (meaning, value, why,
   alternatives + evidence, risks), fingerprint-discipline primer.
   Entry ids (`E-gpu.tensor_split`) anchor probe outputs and the drift
   audit.
2. `libvitriol/src/synth.rs` — two-scale synthetic GGUF generator
   (real shapes from the target GGUF header, dummy weights; reuses the
   rewrite.rs typed-tensor write machinery) + `vitriol probe hardware`
   (per-GPU VRAM/CC, RAM/zram/swap burst, model-store read bandwidth →
   load-time estimate) + `scripts/vitriol` dispatch.
3. `vitriol probe load` — llama-bench grid over `ubatch × kv-quant ×
   mtp-draft` on the synthetic models, NVML VRAM-peak sampling, JSON +
   human reports to `~/.vitriol/probes/`.
4. `vitriol probe recommend --model <gguf>` — estimator sweep over
   candidate configs (tensor_split ratios, ngl, ubatch, KV quant,
   draft_n_max, cache_ram, pin) combined with the measured grid; ranked
   table with measured/extrapolated confidence labels and codex links;
   `--write-profile <name>` writes a profile file, never the live config.
5. Drift audit appended to the codex: live vs blessed table (ubatch,
   adaptive_k, cache_reuse, expert_count, pin_first_n_layers, …), each
   delta with hypothesis + exact commands + pass/fail bar + rollback
   profile.
6. TUI Config view — parse `~/.vitriol/config` (flat `key = value` under
   `[sections]`) at Vitriol-tab entry / `r`; render key = value +
   one-line explanation from a static Rust dictionary; coverage test
   asserts every live key has an entry; risk keys amber; read-only.

## Invariants

- **I1 shapes-real, weights-dummy** — probes never load real model
  weights; synthetic GGUFs carry the real model's architecture/shape
  metadata so KV and compute math are representative.
- **I2 honest confidence** — recommendations label every row as measured
  or extrapolated; never silently mix.
- **I3 fingerprints** — every probe report records the command argv
  (fingerprint discipline).
- **I4 recommend is print-only** unless `--write-profile <name>`;
  never touches the live config.
- **I5 constants-free estimator** — the VRAM model takes shapes from the
  synth/real header; no hardcoded model constants.
- **I6 read-only TUI view** — the Config view never writes the config.
- **I7 coverage** — the TUI dictionary covers every key present in the
  live config (enforced by a test reading the real file).

## Gates

- Stage 1: every live key covered by a codex entry (checked against
  `~/.vitriol/config`).
- Stage 2: `cargo test` (synthetic write → gguf read-back → ModelInfo
  matches spec) + real `llama-bench` smoke on the tiny model.
- Stage 3: grid smoke (tiny model, 2×2 grid) with report written.
- Stage 4: recommend smoke against the real GGUF header (no model load).
- Stage 5: audit table complete, protocols executable as written.
- Stage 6: cargo tests + rebuild + install.
