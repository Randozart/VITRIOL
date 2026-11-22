# Inkling 2 Mini — arch port RESEARCH (PIPELINE phase 1, 2026-11-21 18:40)

Owner directive: *"Let's pull the model, and prep the arch for it."*
Scope: bring Inkling 2 Mini to the in-tree VITRIOL engine (frozen fork) as a
loadable arch, plus download the GGUF. This note is the phase-1 evidence base;
the CONFIRM GATE follows it.

---

## R1 — The model (confirmed from the checkpoint itself)

Parsed the actual GGUF header off the wire (`Range: 0-131071` of
`ggml-org/Inkling-2-Mini-GGUF/Inkling-2-Mini-IQ4_NL.gguf`):

| field | value |
|---|---|
| magic | `GGUF` |
| version | 3 |
| n_tensors | 704 |
| n_kv | 63 |
| `general.architecture` | **`inkling2_loop`** |
| `general.name` | `Inkling 2 Mini` |

Header bytes: `47 47 55 46 | 03 00 00 00 | c0 02 … (704) | 3f 00 … (63)`,
then `general.architecture` → length 20 key → **`inkling2_loop`**.

Native HF config (`thinkingmachines/Inkling-2-Mini/config.json`,
Apache-2.0, 19.526 GB NVFP4):
- `hidden_size 2304`, `num_hidden_layers 24`, vocab 201024, heads 24 / kv 8,
  `head_dim 128`
- `n_routed_experts 128`, `num_experts_per_tok 8`, `intermediate_size 1152`
- **`loop_config {core_start 6, core_end 18, num_passes 3, gate_rank 64}`**
- 20 of 24 layers are local (sliding window 1024); `n_layers_per_attn_res_block 6`
- `final_logit_softcapping 30.0`, `qk_l2_clip_threshold 22.627`,
  `sconv_kernel_size 4`, `rel_extent 1024`, `route_scale 8.0`
- vision: `inkling2_vision`, 23 layers, patch 20 → mtmd mmproj
- MTP head: `mtp.safetensors` (3 layers, `markov_rank 256`,
  `target_layer_ids [5,23,41,48]`) — **not in the GGUF**, and not needed for
  a first bring-up
- README: 26.4B total / 3.4B active → **~5.4B effective active** at 3 passes

## R2 — The Q4-class artifact we can actually run

`ggml-org/Inkling-2-Mini-GGUF` (official ggml-org quant, file listing verified
through the HF API):

| file | size | note |
|---|---|---|
| `Inkling-2-Mini-BF16.gguf` | 52.2 GB | no |
| `Inkling-2-Mini-Q8_0.gguf` | 27.8 GB | no (VRAM) |
| **`Inkling-2-Mini-IQ4_NL.gguf`** | **17.6 GB** | imatrix-calibrated; experts L0–19 IQ4_NL, rest Q8_0; loop controller F32; **95.5 % same top token** vs BF16, KL 0.0095 |
| `mmproj-Inkling-2-Mini-Q8_0.gguf` | 346 MB | vision (optional) |
| `mmproj-Inkling-2-Mini-BF16.gguf` | 642 MB | vision (optional) |

Disk: `/home` has **210 G free** → model + mmproj fit comfortably in `~/Models`.
HF CDN stalls on this host (AGENTS.md Testing Protocol §6) → the download must
be a **ranged-resume fetch verified against the tree-API `lfs.oid` sha256**
(*not* the resolve-HEAD etag, which is a xetHash).

## R3 — Upstream support: what actually exists (and when)

`ggml-org/llama.cpp` **does** ship the arch. Five commits, in order:

| commit | title | files |
|---|---|---|
| `5c008e00` | feat: add Inkling 2 support (text + vision) (#32217) | 45 files, +1667 |
| `8af68d48` | llama: fixes for Inkling 2 attention mask (#32429) | llama-graph.cpp **+233**, .h **+64**, inkling2.cpp **+486/−224**, models.h |
| `a1f42226` | model: load ffn_gate_up_exps scales (#32657) | llama-model.cpp +6 |
| `19bc41db` | model: apply `.scale` tensors automatically (#32669) | 24 model files (generic infra) |
| `29b52d39` | model: Inkling 2 Mini (#32694) | llama-arch.cpp/h (+37/+19), `src/models/inkling2-loop.cpp` (new 113), inkling2.cpp **+278**, models.h +59, gguf-py |

Implementation files that matter for inference:
- `src/models/inkling2.cpp` — **457 lines** (hparams, tensors, graph)
- `src/models/inkling2-loop.cpp` — **115 lines** (loop controller tensors)
- `src/llama-graph.h/.cpp` — new `llm_graph_input_attn_rel_pos`,
  `build_attn_iswa_impl` refactor, and a *generic* hybrid-iswa mask-reuse fix
- `src/llama-arch.h` — `LLM_ARCH_INKLING2` / `LLM_ARCH_INKLING2_LOOP`,
  `LLM_KV_LOOP_{START,END,COUNT,GATE_RANK,VARYING}`, and 18
  `LLM_TENSOR_LOOP_*` names (msg_query, {q,k,v,ffn}_ctrl_{down,gain,forget})
- `src/llama-vocab.cpp` — `tokenizer_pre == "inkling2"` → `PRE_TYPE_GPT4O`,
  `special_bos_id = NULL`, `clean_spaces = false`
- `src/llama-quant.cpp`, `llama-model-saver.cpp` — tensor-name registrations
- `tools/mtmd/models/inkling2v.cpp` — vision encoder (optional for milestone 1)

### CUDA delta (the only kernel change)

`5c008e00` touches 5 fattn files. The change is *surgical, not algorithmic*:
- gqa/scan optimizations are gated on **`mask->ne[2] == 1`** (a mask with a
  head dimension is not shared between Q heads)
- mask pointer gains the head offset: `+ nb32*(head % ne32)` (vec) or
  `nb32*(zt_Q % ne32)` (tile)
- `nb31` parameter widens `int32_t` → keep `int32_t`, `nb32` widens to `int64_t`
- logit-softcap kernels are restricted to `ne[0] == 128 || 256`
- `kv_rows` compacted masks return `BEST_FATTN_KERNEL_NONE`

**No new ggml op is introduced by any of the five commits.**

## R4 — Fork readiness (measured, not assumed)

| requirement | fork status | evidence |
|---|---|---|
| `src/models/` refactor present | **YES** | `engine/src/models/` has 60+ arch files, e.g. `arcee.cpp` |
| model API shape (`llama_model_base`, `load_arch_hparams`/`load_arch_tensors`, `LLAMA_LOAD_LOCALS`, `create_tensor_qkv`) | **YES** | `engine/src/models/arcee.cpp:1-40`, `engine/src/llama-model.h:780` |
| hybrid-iswa graph infra (`llama_memory_hybrid_iswa_context`, `llm_graph_input_attn_kv_iswa`, `llm_graph_input_rs`) | **YES** | `engine/src/llama-graph.{h,cpp}` |
| `llm_graph_input_attn_rel_pos` (new in `8af68d48`) | **NO — must add** | grep: absent |
| `build_attn_iswa_impl` refactor | **NO — must add** | grep: absent |
| generic `.scale` per-tensor load pass | **YES, already** | `engine/src/llama-model.cpp:1583` — *"generic pass: load optional per-tensor/per-expert '.scale' tensors (e.g. NVFP4 scale2)"*
| `ggml_ssm_conv`, `ggml_scale_bias`, `ggml_fill` | **YES** | all three in `ggml/include/ggml.h` + `ggml/src/ggml.c` |
| GGML op enum size | fork **107** mentions vs upstream **105** | no upstream ops missing |
| `LLM_KV_NUM_LOOPS` + a looped-model precedent | **YES** | `llama-arch.cpp:246`, used by `src/models/nanbeige.cpp:7` |
| `build_attn` overload set | **same ordinals** (1195/1211/1227 vs upstream 1343/1359/1375) | `llama-graph.h` |
| tokenizer pre-type `GPT4O` | present (used widely) | `llama-vocab.cpp` |
| `tests/test-llama-archs.cpp` | present (877 lines vs upstream 2016) | `engine/tests/` |

Divergence on the files the port touches (fork vs upstream line counts):
`llama-graph.cpp` 3937/4756 (**−17 %**), `llama-graph.h` 1391/1591 (−13 %),
`llama-model.cpp` 3410/4345 (−21 %), `models.h` 2628/3422 (−23 %),
`llama-arch.cpp` 152 archs registered.

**Consequence:** this is a *hand-adapted transplant*, not a cherry-pick. The
`src/models/inkling2*.cpp` files are close to drop-in (same base class API);
`llama-graph` additions must be re-expressed against the fork's (older, smaller)
iswa/attn code; the CUDA fattn deltas must be rebased onto the VITRIOL-modified
fattn kernels.

## R5 — Attention path: can milestone 1 avoid the CUDA change?

`src/models/inkling2.cpp` contains **no `cparams.flash_attn` branch** — the
model goes through `build_attn`, so the FA / non-FA decision stays in the
shared graph. `ggml_soft_max_ext` in the fork accepts an arbitrary mask tensor.

So milestone 1 can run **`-fa off`** (plain `ggml_soft_max_ext` with the 4-D
per-head bias mask) and defer the fattn head-mask rebase to milestone 2, where
it is measured against the non-FA baseline. This splits the highest-risk item
out of the critical path.

## R6 — What the vendor claims / what to expect locally

Vendor table (README §5) vs the current driver Qwen3.8-27B: wins SWEBench Pro
77.9/73.8, DeepSWE 48.2/42.2, PMPP-Hard 455.8/262.5, BrowseComp 79.0/61.6,
Vibe Code 77.9/64.9; loses Terminal-Bench-in-pi 71.6/73.0 and non-hallucination
63.4/71.4. README itself warns of "confident hallucination" and "repetitive
reasoning loops" and recommends output/turn caps.

Local reality to measure (not assume): IQ4_NL 17.6 GB across 3060 (12 G) +
1070 Ti (8 G) is *just* feasible at 16–32 K ctx with q4_0 KV; looping costs
~2× compute/token; sm_61 kernel coverage is untested for sconv / rel-pos bias.
Acceptance bar for "worth keeping": **≥ 8 t/s at 16 K filled** and no crash at
the declared window.

## R7 — Risks ranked

1. **llama-graph rebase** (moderate–high): the rel-pos input touches the same
   iswa/attn input code that the fork diverged from by ~17 %. Failure mode:
   silent mask misalignment → gibberish that still "runs".
2. **CUDA fattn rebase** (moderate): VITRIOL's fattn is modified; a wrong merge
   could regress *existing* models. Mitigated by doing it in its own stage with
   a von existing-model regression run (27B golden prompt).
3. **Pascal (sm_61) coverage** (medium): no community data; fallback is to run
   the model on the 3060 alone (`exclude_secondary = true`), which also halves
   the ts fitting problem.
4. **Performance** (medium): may land single-digit t/s → not a daily driver,
   but a valid *capability lane* evaluated by dream-eval.
5. **Vision** (low, deferred): mtmd `inkling2v.cpp` is an extra port; and
   `vis=on` disables slot persistence in VITRIOL.
6. **Overfitting to the vendor table** (low): our own dream-eval on the corpus
   is the only number that decides promotion.

## R8 — Port checklist (derived, ordinal)

Engine (`engine/`):
1. `src/llama-arch.h` — `LLM_ARCH_INKLING2`, `LLM_ARCH_INKLING2_LOOP`,
   `LLM_KV_LOOP_*`, `LLM_TENSOR_LOOP_*`, `LLM_TENSOR_ATTN_REL_*`,
   `LLM_TENSOR_SCONV_*` as upstream defines them
2. `src/llama-arch.cpp` — `LLM_ARCH_NAMES` (`"inkling2"`, `"inkling2_loop"`),
   `LLM_KV_NAMES`, `LLM_TENSOR_NAMES` entries, tensor-name switch cases
3. `src/llama-vocab.cpp` — `tokenizer_pre == "inkling2"` branch
4. `src/llama-hparams.cpp/h` — `sliding_window` per-layer pattern, rel extent
   (only if inkling2.cpp needs hparams fields the fork lacks)
5. `src/models/models.h` — `llama_model_inkling2`, `llama_model_inkling2_loop`
6. `src/models/inkling2.cpp`, `src/models/inkling2-loop.cpp` — transplanted
7. `src/llama-model.{h,cpp}` — dispatch cases (`LLM_ARCH_INKLING2*`)
8. `src/llama-quant.cpp` (+ saver) — tensor names for quantisation/round-trip
9. `src/llama-graph.{h,cpp}` — `llm_graph_input_attn_rel_pos`,
   `build_attn_iswa_impl`, iswa mask-reuse fix
10. `ggml/src/ggml-cuda/fattn*` — head-dimension mask support *(stage 2)*
11. `tools/mtmd/models/inkling2v.cpp` *(deferred)*
12. `tests/test-llama-archs.cpp` — arch cases

VITRIOL (root):
13. `profiles/inkling-2-mini-iq4nl-16k/{config,meta}`
14. `scripts/fetch-gguf.py` (ranged-resume + tree-API sha256 verify) if not
    already present
15. `.opencode/plans/inkling-2-mini-port-2026-11-21.md` (phase 3 deliverable)

## Sources
- `huggingface.co/thinkingmachines/Inkling-2-Mini` (config.json, README §3/§5/§7)
- `huggingface.co/ggml-org/Inkling-2-Mini-GGUF` (README, file listing, GGUF header)
- `github.com/ggml-org/llama.cpp` commits `5c008e00`, `8af68d48`, `a1f42226`,
  `19bc41db`, `29b52d39` (cloned at `/scratch/briev-builds/llama-upstream`, HEAD `8a97fa9`)
- Local: `engine/src/llama-arch.cpp`, `engine/src/models/{models.h,arcee.cpp}`,
  `engine/src/llama-graph.{h,cpp}`, `engine/src/llama-model.cpp:1583`,
  `engine/ggml/include/ggml.h`, `docs/ENGINE_PROVENANCE.md`
