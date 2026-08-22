# Fine-tuning feasibility — D2 forge scoping findings

**Date:** 2026-08-22 08:00
**Status:** findings recorded; D2 deferred until harvest volume justifies compute
**Trigger:** review of external fine-tuning advice (SFT/DPO/LoRA on Mellum2;
Qwen3.8 architectural-surgery proposals)

## Verdicts

### Mellum2 LoRA/DPO on harvested data — sound concept, hardware-blocked locally

- Unsloth Feb-2026 MoE kernels support Qwen3/gpt-oss/DeepSeek/GLM only —
  `mellum` arch not yet listed; generic PEFT falls back to slow paths.
- **4-bit QLoRA unsupported for MoE** (bitsandbytes limitation) → bf16 LoRA only.
- bf16 LoRA VRAM reference: Qwen3-30B-A3B ≈ **63 GB** (same size class as
  Mellum2 12B). Local 3060/12 GB cannot host it.
- Paths when justified: rent one spot A100/H100 hour per training run (~$1–3,
  weights+data stay sovereign); revisit Unsloth `mellum` support upstream; or
  test generic Transformers-v5 PEFT acceptability.
- **Sanctioned base found**: JetBrains released
  [`Mellum2-12B-A2.5B-Thinking-SFT`](https://huggingface.co/JetBrains/Mellum2-12B-A2.5B-Thinking-SFT)
  — the intermediate checkpoint before their RL stage, published explicitly for
  post-training experiments (preference tuning, RLVR). Use this, not a
  community quant, as the D2 base when the time comes.

### Qwen3.8 architectural surgery — rejected

| proposal | verdict | reason |
|---|---|---|
| prune 16→4 KV heads + recovery SFT | never | solves nothing (measured q4_0 KV @64k ≈ 0.7 GB); deleting 75% of attention capacity needs billions of recovery tokens, not 50–100k |
| retrofit SWA onto global-attention layers | never | model trained expecting full context; masking degrades long-range retrieval by construction |
| graft Medusa/Eagle MTP head | redundant | our fork already runs Qwen3.8's native MTP head (`--spec-type mtp`); deeper-than-depth-1 already measured as regression |

Several cited tools/benchmarks in the reviewed material (GSpark decoding,
NVFP4+SGLang configs) do not match our llama.cpp stack and appear synthetic.

### Adopted cheap wins

- Thinking-control for Qwen in agent loops (`enable_thinking:false`,
  effort knobs) — already proven for patch drafting; extend to hermes-brain
  turns and verifier calls where latency matters more than deliberation.
- Prefix caching is safe + effective post-H1 gate (26.9s → 1.2s measured).

## Standing decisions

1. D1 harvesting continues regardless — datasets outlive hardware decisions.
2. D2 training deferred until distill volume justifies one rented GPU hour;
   base = JetBrains Thinking-SFT checkpoint; dataset = ~/.vitriol/distill/.
3. Surgery proposals filed under "not on my watch".
