# Inkling 2 Mini arch port — PLAN (2026-11-22)

**Goal**: bring `inkling2_loop` (Inkling 2 Mini) into the frozen VITRIOL engine fork as a
loadable arch, load the IQ4_NL GGUF on CPU, produce coherent text, and leave every other
arch byte-for-byte unaffected. The live dual-slot 27B engine is untouched; all work is on
engine source + a throwaway smoke profile.

**Upstream reference** (extracted to `/scratch/briev-builds/inkling-port/`):
commits `5c008e00`, `8af68d48`, `a1f42226`, `19bc41db`, `29b52d39` of
`ggml-org/llama.cpp` (HEAD `8a97fa9`). PROVENANCE notes on each touched engine file:
`// PROVENANCE: ported from ggml-org/llama.cpp <shas> ("<title>") — <what>`.

## Stages (one commit each)

**C1 — arch registration** (`llama-arch.h/.cpp`)
- enums: `LLM_ARCH_INKLING2`, `LLM_ARCH_INKLING2_LOOP`
- KVs: `VOCAB_SIZE_UNPADDED`, `ATTENTION_RELATIVE_LENGTH/EXTENT`, `QK_CLIP`,
  `RELATIVE_CLIP`, `LOG_SCALE_FLOOR`, `LOG_SCALE_ALPHA`,
  `LOOP_START/END/COUNT/GATE_RANK/VARYING` (with `LLM_KV_NAMES` map entries)
- tensors: 17 attn/ffn names + 13 loop names (`LLM_TENSOR_NAMES` + `LLM_TENSOR_INFOS`
  with op kinds from the upstream diff)
- arch predicates: is_hybrid, rs_rollback, mixed_batch (false), sm_tensor,
  supports_save (INKLING2_LOOP → false, loop cache is expanded at load)
- hparams fields: `n_sconv_channels`, `expert_weights_norm_eps`, `expert_router_f32`,
  `n_relative`, `n_relative_extent`, `n_vocab_unpadded`, `n_vocab_padded`,
  `n_log_scale_floor`, `f_log_scale_alpha`, `f_qk_clip`, `f_relative_clip`
  (+ `n_embd_r()` sconv branch)
- gate: `engine` builds; `test-llama-archs` round-trips both arch names.

**C2 — model files + dispatch**
- `src/models/models.h`: `llama_model_inkling2`, `llama_model_inkling2_loop` decls
  (layer-struct fields: `attn_rel`, `attn_rel_bias`, `attn_rel_clip`, `attn_q_clip`,
  `attn_sink_proj`, `attn_{q,k,v}_inp_norm`, `attn_{k,v}_conv`, `attn_{k,v}_conv_proj`,
  `attn_post_conv`, `attn_res_query`, `output_res_query`, `ffn_post_conv`,
  `ffn_res_query` + 13 loop tensors on the loop model)
- `src/models/inkling2.cpp` (457 L), `inkling2-loop.cpp` (115 L) — transplanted, minus
  `prec_policy.set_acc` (fork has no prec_policy API; drop, output acc default)
- `llama-model.cpp`: `llama_model_mapping` cases; `create_memory` filter for both archs;
  `llama_model_rope_type` → NONE; adapt the `a1f42226` ffn_gate_up_exps-scale hunk to
  the fork's generic `.scale` pass (llama-model.cpp:1583)
- `llama-model-saver.cpp` + `llama-quant.cpp`: tensor-name registrations (upstream diff)
- `llama-vocab.cpp`: `tokenizer_pre == "inkling2"` → GPT4O pre-type, BOS=NULL,
  clean_spaces=false
- gate: engine builds; `llama-quantize` can map the names; arch test still green.

**C3 — graph rel-pos input**
- `llama-graph.h/.cpp`: `llm_graph_input_mem_hybrid_iswa_rel` (per-head distance
  bias), `build_attn_iswa_impl` split, iswa mask-reuse fix (from `8af68d48`)
- gate: engine builds; existing models' graphs unchanged (they don't touch the new
  input); C2/C3 smoke: load the IQ4_NL GGUF on CPU (`-fa off --n-gpu-layers 0`),
  0 missing tensors, coherent English at 512 ctx.

**C4 — tests + report**
- `tests/test-llama-archs.cpp`: inkling2 + inkling2_loop cases (upstream test diff)
- regression run on the live 27B (fingerprint + golden prompt, see I1)
- report `.opencode/plans/inkling-2-mini-arch-port-2026-11-22.md` with the measured
  t/s at the declared window; decision on VITRIOL profile (separate work, not in this
  grind unless t/s ≥ 8)

**C5 — Rust: arch-aware VRAM estimator** (`libvitriol/`, per owner directive "use Rust where possible")
- The engine arch port (C1–C4) is **C++ by necessity**: it lives inside the frozen
  ggml fork and there is no FFI path into ggml graph construction. This stage is the
  Rust surface of the effort.
- `libvitriol/src/estimator.rs`: add an arch-aware KV correction — when GGUF
  `general.architecture == inkling2_loop`, `kv_mib_per_token` must reflect hybrid iswa
  (attention-res blocks every 6 layers → only ~⅙ of layers hold full KV) + sconv state,
  not the flat `hd × head_count_kv × 2.5`. Keyed off `ModelInfo` (arch string +
  `n_layer_kv`), unit-tested in isolation against the Inkling-2 GGUF dims.
- `libvitriol/src/gguf.rs`: confirm packed top-8 expert tensors classify into
  `per_layer_experts_bytes` correctly (add a fixture test).
- gate: `cargo test` green in `libvitriol/`; `vitriol calibrate` prints the corrected
  KV figure for the Inkling-2 GGUF. No change to the 27B estimate (parity — the arch
  correction only fires for `inkling2*`).

## Invariants

- **I1 — other archs unchanged.** With no inkling GGUF loaded, the 27B model loads with
  byte-identical `VITRIOL-FINGERPRINT` and byte-identical greedy output on a fixed 32-token
  prompt. Pinned by: fingerprint diff before/after + `tests/test-llama-archs` full suite.
- **I2 — arch-string round-trip.** `inkling2`/`inkling2_loop` map to the right `llm_arch`
  and back; unknown strings still fail. Pinned by: `test-llama-archs` cases.
- **I3 — zero new ggml ops.** `ggml/include/ggml.h` GGML_OP count unchanged (107
  mentions); the port only composes existing ops. Pinned by: grep count + build.
- **I4 — tensor coverage.** The IQ4_NL GGUF loads with **0 missing tensors** (loader
  reports every one of the 704 tensors). Pinned by: load-log assertion in the C3 gate.
- **I5 — text-only load.** Loading without `--mmproj` must succeed (vision tensors are
  TENSOR_NOT_REQUIRED / separate arch). Pinned by: the C3 smoke command omits `--mmproj`.
- **I6 — CPU/CUDA parity (stage 2, later).** Greedy 64-token decode identical between
  `-fa off --n-gpu-layers 0` and the CUDA build. Pinned by: a diff script comparing
  token streams (added when stage 2 lands).
- **I7 — VITRIOL probe untouched.** No VITRIOL config change ships in this port; the
  smoke uses `llama-cli`-equivalent direct launches. Pinned by: `git diff --stat`
  showing no `~/.vitriol/config` / `profiles/` / `scripts/vitriol` edits in C1–C4.
- **I8 — honesty.** The report quotes measured t/s and measured tensor counts; no
  interpolated claims. Pinned by: report file content + the gate logs it links.
- **I9 — estimator parity (Rust).** The arch-aware KV correction fires ONLY for
  `inkling2*` architectures; every other arch (incl. the 27B daily driver) yields a
  byte-identical `VramEstimate`. Pinned by: a `libvitriol` unit test asserting the 27B
  estimate is unchanged pre/post C5, plus a new `inkling2_loop` case asserting the
  corrected KV figure.
