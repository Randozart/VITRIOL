# Inkling 2 Mini as daily driver — RESEARCH (2026-11-21 17:55)

Owner question: could **Inkling 2 Mini** (share.google/3S5IxsbT3nS4Pl2PT,
"Cutting-edge Performance for your Local Setup") become the daily driver?
This is a RESEARCH note (no code changes). Verdict at the end.

## 1. What it is

**Thinking Machines** (Mira Murati's lab) released **Inkling 2 Mini**, a
**multimodal looped transformer** (first looped release from the lab).

- HF: `thinkingmachines/Inkling-2-Mini` — license **Apache-2.0**
  (huggingface.co/thinkingmachines/Inkling-2-Mini).
- Architecture (from `config.json`):
  - `model_type: inkling2_loop`, arch `Inkling2LoopForConditionalGeneration`
  - **26.4B total / 3.4B active** (README §3; config has `hidden_size: 2304`,
    `num_hidden_layers: 24`, vocab 201024, `n_routed_experts: 128`,
    `num_experts_per_tok: 8`)
  - **Looped**: middle 12 layers (ids 6–17, `loop_config.core_start/end`) run
    **3 passes with shared weights** → **48 layer applications per token**
    from 24 layers of weights → **5.4B effective active params/token**.
  - Hybrid attention: 20 local (sliding-window 1024) + global layers,
    attention-residuals block every 6 layers, logit softcapping 30.0.
  - **Multimodal**: native vision tower (`inkling2_vision`, 23 layers,
    patch 20px), mmproj ships separately (642 MB BF16 / 346 MB Q8_0).
  - **MTP draft head** (`mtp.safetensors`, 3 layers) — speculative decoding
    support in the reference stack.
  - **1M context** (`model_max_length: 1048576`).
  - `reasoning_effort` chat-template knob: none…max / 0.0–0.99;
    minimum effort does NOT fully disable reasoning (README §7).

## 2. Native numerics vs what runs on llama.cpp

- The native checkpoint is **NVFP4-quantized** (modelopt) — 19.5 GB on disk;
  numerics support is **NVFP4 and MXFP8** (expert storage per-layer).
  NVFP4 needs Blackwell (sm_120+); MXFP8 needs sm_89+. **Neither runs on this
  rig** (RTX 3060 sm_86 / GTX 1070 Ti sm_61).
- Practical path is **GGUF** via `ggml-org/Inkling-2-Mini-GGUF`
  (verified file listing):
  | quant | size | notes |
  |---|---|---|
  | BF16 | 52.2 GB | NVFP4+MXFP8 experts upcast — no |
  | Q8_0 | 27.8 GB | near-lossless, 97.6% same top token vs BF16 — no fit |
  | **IQ4_NL** | **17.6 GB** | imatrix-calibrated; experts L0-19 IQ4_NL, rest Q8_0; 95.5% same top token — **candidate** |
  | mmproj | 642 / 346 MB | vision encoder |
- README says llama.cpp support exists (via ggml-org build); `llama serve
  -hf ggml-org/Inkling-2-Mini-GGUF:IQ4_NL`.

## 3. Benchmarks (vendor table, README §5 — self-reported, treat as claims)

vs Qwen3.8-27B (the current driver), Gemma 4 31B, Muse Glimmer, Nemotron 3.5:
- **SWEBench Pro v2 (Hard): 77.9 vs 73.8** (best in row)
- **SWEBench Verified (bash): 79.2 vs 77.8** (best)
- **SWEBench Multilingual: 73.7 vs 70.0** (best)
- **DeepSWE v1.1: 48.2 vs 42.2** (best)
- **FrontierCode v2.0: 15.1 vs 14.9** (best)
- **Terminal Bench v2.1 (pi v0.83): 71.6 vs 73.0** (Qwen best)
- **Vibe Code Bench v1.1: 77.9 vs 64.9** (best)
- **PMPP-Hard (pi): 455.82 vs 262.50** (best, large margin)
- **BrowseComp (compaction): 79.0 vs 61.6** (best)
- **AA Index v5: 20.1 = 20.1** (tied with Qwen)
- **Vals Index: 46.5%** (best)
- **Apex 2025: 34.4 vs 18.8**; ARC-AGI v2: 49.3 vs 42.4
- **HLE-Diamond: 15.2 vs 15.3** (Qwen marginally best)
- **AA-Omniscience non-hallucination: 63.4 vs 71.4** (Qwen better)
- **Mistral-class safety**: Misbehavior 9.8% (best, lower=better)

Vendor framing: "less capable than Inkling 2 Small across most evaluations;
largest gaps on factual recall; **more likely than larger models to state
incorrect information with confidence**".

## 4. Fit for this rig (RTX 3060 12 GB + GTX 1070 Ti 8 GB, sm_86/sm_61)

- **IQ4_NL = 17.6 GB** fits *just* across ts 27,21 (weights ~18.5 GiB
  incl. mmproj option) with q4_0/q8_0 KV at modest ctx; 1M ctx is out
  (KV at 1M on a 24-layer MoE ≈ 20+ GiB). Realistic: **32K–64K window**,
  same shape as the 208K-fallback profile territory.
- **Throughput is the real question**: the loop makes each token cost
  3× the middle 12 layers + full 24 → roughly **2× FLOPs/token** vs a
  same-size dense model, with 5.4B effective active params. On
  DDR3/PCIe Pascal+Turing, expert fetch dominates. Expect **single-digit
  t/s** on this dual-GPU pair — plausibly 3–8 t/s at moderate depth, i.e.
  materially *slower* than the live 27B (16.5 t/s) even before KV
  pressure.
- **VRAM headroom**: 3060 currently 1016 MiB free, 1070 Ti 837 MiB free with
  the 27B resident. A 17.6 GB model across ts 27,21 leaves ~3 GiB for
  KV/compute per GPU — tight but workable at 32–64K ctx with q4_0 KV.
- **Pascal (sm_61) kernels**: the fork builds for "61;86"; Inkling's exotic
  layers (loop controller F32, sconv, attention-residuals) are untested on
  sm_61 in the llama.cpp community so far — expect a risk of missing or
  slow kernels; VMM/mmap off is required as with the 27B.

## 5. Integration surface (what it would touch)

- **llama.cpp arch support**: the in-tree fork's `llama-arch.cpp` (152
  archs, `LLM_ARCH_*` list) does **NOT** contain an inkling entry as of
  inner commit (upstream sync frozen 2026-10-23). Upstream llama.cpp has
  added `inkling2_loop` support (that's what ggml-org GGUF targets). So
  **daily-driver status requires an engine change**: port the arch (24-layer
  looped MoE, sconv, attn-res, MTP, vision mmproj) into the fork — a real
  porting effort, not a config swap. This is the single largest blocker.
- **Profiles**: a new `profiles/inkling-2-mini-<ctx>` would follow the
  existing shape (config + meta), ts 27,21, q4_0 KV, `mmap=off`.
- **Caduceus routing**: already present in the catalog as
  `baseten/thinkingmachines/inkling` and `baseten/thinkingmachines/inkling-small`
  (cloud, `~/.vitriol/officina/caduceus-models.json` — unrelated API IDs,
  not the local GGUF). The local lane would be a new virtual lane pointing
  at the engine under a new alias.
- **MTP**: ships `mtp.safetensors`; whether the fork's MTP engine accepts a
  third-party MTP layout (markov_rank 256, target_layer_ids [5,23,41,48])
  is unknown — likely no, another port gap.
- **Chat template**: custom (`<|message_user|>` etc. with `reasoning_effort`
  system injection) — needs the template file shipped in `profiles/`
  (the 27B already uses a custom `chat-template-tolerant.j2`, so the
  mechanism exists).

## 6. Failure modes / risks

1. **Engine port is the gate.** No inkling arch in the frozen fork →
   model cannot load at all until ported. Upstream support is recent;
   porting a looped-MoE + vision + MTP arch is days of work and will
   re-introduce CUDA kernel surface on sm_61.
2. **Throughput regression.** 2× compute/token; on DDR3 Pascal the
   expert bandwidth wall is worse than for the 27B. The vendor's own
   numbers are at 32 GB+ single-GPU; no local dual-GPU Pascal data exists.
3. **Factual reliability.** Vendor admits more confident hallucination vs
   larger models; AA-Omniscience non-hallucination 63.4 vs Qwen 71.4.
   For a coding agent that *executes tools*, the safety row (destructive
   actions 10.2% vs 35.2%) looks strong, but the hallucination row is
   the one that bites in a long autonomous loop.
4. **KV/depth wall.** 1M ctx advertised; realistic here 32–64K. The
   current driver runs 262K; an agent workload that needs 100K+ filled
   context loses ground.
5. **KV-unified dual-slot**: the 27B's kv_unified dual-slot setup
   (parallel=2, 2×262K) would not translate — the looped KV (each pass
   keeps its own KV cache) multiplies KV cost per pass; dual-slot at 32K
   each is the realistic ceiling.
6. **Vision**: mmproj is optional and adds 350 MB–0.64 GB; persistence
   (slot save/restore) is disabled while multimodal is loaded (the
   `vis=on` fingerprint rule) — so enabling vision would shut off the
   checkpoint chain.
7. **Reasoning-effort plumbing**: the `reasoning_effort` knob is
   template-level; the fork's `reasoning_guard()` refuses
   `reasoning!=off` with `reasoning_budget=-1` (scripts/vitriol) — a
   config conflict to resolve before it can even load.

## 7. What it would NOT beat

- The 27B BuffedMod (Q6K) already sits at 16.5 t/s with 262K window,
  MTP n=2, sparse probe KV, dual-slot — a heavily optimized operating
  point. Inkling 2 Mini's advantage rows (SWEBench Pro, DeepSWE, PMPP,
  BrowseComp) are **harness-heavy agentic benchmarks** that also reward
  the pi/OpenHands scaffolding, not raw model quality.
- Free-inference lanes (deepseek-v4.1-flash etc.) already carry the
  heavy reasoning in the cascade; a slower local model changes the
  economics of the local-first tier.

## 8. Verdict & recommended next steps

**Verdict: not a daily-driver candidate TODAY.** The capability story is
genuinely strong (multiple best-in-row agentic results vs the 27B), but:

1. The in-tree engine **cannot load it** without an arch port (biggest
   blocker; upstream support exists, fork is frozen).
2. Even after a port, **throughput on sm_86+sm_61 is unproven and likely
   a regression** (2× compute/token, 17.6 GB weights, Pascal bandwidth
   wall).
3. The 1M-ctx claim doesn't survive contact with 20 GiB combined VRAM.

**Reasonable middle paths (ranked):**
1. **Cloud lane first**: `baseten/thinkingmachines/inkling(-small)` is
   already in the Caduceus catalog — wire it as a **free/cloud overflow
   lane** via the existing probe machinery (zero engine work, measurable
   via `/dream-eval --lanes`). This captures the capability upside
   (PMPP/DeepSWE rows) without the port.
2. **GGUF side-by-side smoke test** (NOT replacing the 27B): download
   `ggml-org/Inkling-2-Mini-GGUF:IQ4_NL` (17.6 GB) to `~/Models`, run a
   *standalone* `llama.cpp` (upstream, not the fork) build on the 3060
   alone with `-ngl 99` at 16–32K ctx, and bench t/s + a short
   dream-eval run. Gives a local number to decide whether the engine
   port is worth it. ~1 h of work, no VITRIOL changes.
3. **Engine port** only if (2) shows ≥ ~10 t/s and the capability gain
   on the dream-eval corpus justifies it. That's a separate pipeline
   (arch port + sm_61 kernels + MTP layout + chat template).

**Not recommended:** swapping the 27B out of the live config now — no
loadable path exists in the fork, and the resume/KV/persistence stack is
tuned to the current fingerprint.

## Sources
- huggingface.co/thinkingmachines/Inkling-2-Mini (config.json, README.md,
  model.safetensors.index.json — 19.526 GB NVFP4)
- huggingface.co/ggml-org/Inkling-2-Mini-GGUF (README.md + file listing)
- thinkingmachines.ai/news/inkling-2-mini (linked from README)
- Local: `engine/src/llama-arch.cpp` (152 archs, no inkling entry),
  `~/.vitriol/officina/caduceus-models.json` (baseten inkling entries),
  `scripts/vitriol` (fingerprint + reasoning_guard), `profiles/` layout.
