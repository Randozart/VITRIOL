# LoRA Try-A: rico03 Opus-Reasoning Distill on the Daily Driver

**Date**: 2026-10-12 (session ~10:00–11:00 CEST)
**Status**: COMPLETE — adapter converts, loads, runs; owner decision pending
**Scope**: Option A from the community-adapter survey — evaluate the only
credible shared 3.8-base reasoning LoRA (`rico03/Qwen3.8-27B-Claude-Opus-Reasoning-Distilled`,
Apache-2.0) against the live daily driver. No knowledge injection (owner
directive).

## Provenance

- Adapter source: `rico03/Qwen3.8-27B-Claude-Opus-Reasoning-Distilled`
  - `adapter_model.safetensors` 933,974,032 B,
    sha256 `aec716c1700f8f796226ed54e75aa03c77241d73d5de4d5bd53882e78960ffe4`
    (verified against tree-API `lfs.oid` via `scripts/fetch-hf-ranged.py`)
  - PEFT config: r=32, alpha=32 (scale 1.0), no rslora, no alpha_pattern,
    `unsloth_fixed: true`, regex targets all attn + GDN in/out projections + MLP
  - 992 GGUF tensors: 16 full-attn layers (attn_q/k/v/output ×2) + 48 GDN
    layers (attn_qkv/attn_gate/ssm_beta/ssm_alpha/ssm_out ×2) + 64 MLP
    (ffn_gate/up/down ×2) — matches 64 blocks, full_attention_interval 4
- Converted adapter (f16, 466,979,552 B):
  `~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf`
- Baseline arm = live unit UNTOUCHED:
  `VITRIOL-FINGERPRINT: unit pid=181279` (qwen38-buffedmod-262k blessed set:
  ts 27,21, c 262144, MTP n=3, q4_0 KV, fa on, cram=12288, ckpts=4,
  `--reasoning off`, port 8279)
- Adapter arm = manual launch, identical argv + `--lora
  /home/randozart/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf`, identical
  env (all VITRIOL_*/GGML_* captured from `/proc/181279/environ`):
  `VITRIOL-FINGERPRINT: manual-eval-launch adapter-arm argv=unit-181279-argv+--lora(...) ts=27,21 ctx=262144 spec=mtp:3 kv=q4_0/fa ckpts=4 cram=12288`
- Sampling: temperature 0, top_k 1, max_tokens 400, `/v1/chat/completions`,
  12 prompts (6 language-design, 6 systems-programming) —
  `/tmp/opencode/lora/eval/{prompts.py,run_eval.py}`
- Unit stopped for adapter arm (`systemctl stop`), restored immediately after
  (`active` re-verified). Manual process killed by explicit PID (439780).

## Converter fix (inner commit `cd55f1f3a`)

`convert_lora_to_gguf.py` crashed on this adapter:
`_LinearAttentionVReorderBase._reorder_v_heads(..., dim=1)` (GDN out_proj
column reorder, `conversion/qwen.py:616`) hit
`LoraTorchTensor.reshape`'s `NotImplementedError` — the wrapper only permits
last-dim-preserving reshapes, and every Qwen3.5-class hybrid adapter with
out_proj targets hits this path.

Fix: `LoraModel._reorder_v_heads` override —
- row-side (dim 0): reorder lives in `lora_B`; delegate to the plain reorder
  on B (W' = (QB)A = Q(BA), exact by associativity)
- col-side (dim 1): reorder lives in `lora_A`; `W' = (BA)P = B(AP)` — permute
  A's input dim only

Numeric verification (torch, layer 0 out_proj, nk=16 v_per_k=3 hd=128):
`max abs diff(B@perm(A), reorder_cols(B@A)) == 0.0` exactly; unpermuted
sanity diff 2.1e-3 (permutation real). Row-side exact by construction.
Also required `transformers>=5` (`qwen3_5` config class; 4.57.6 predates it —
installed 5.19.0 in `/tmp/opencode/loraenv`).

## Runtime result

- Adapter loads at startup, `/lora-adapters` → id=0 scale=1.0, applied to all
  requests. Load time unchanged (~230 s to health-200).
- No fork changes needed for inference: existing `--lora`
  (`common/arg.cpp:2984`) works as-is on the IQ3_S base.

### Speed (predicted t/s, 400-token greedy, shallow ~70-token prompts)

| arm | mean predicted t/s | range |
|---|---|---|
| base (unit) | 26.4 | 23.8–28.4 |
| adapter | 21.9 | 19.7–27.5 |

≈ −17% decode. Expected, two components: (1) known gap — MTP draft `ctx_dft`
is never adapter-primed (`common/speculative.cpp:2609` lacks
`llama_set_adapters_lora`) so acceptance drops while the target corrects;
(2) adapter mul_mat overhead on hooked layers. The ctx_dft patch is the
recovery path and was NOT part of stage A. Prompt-side cost is negligible
after warmup.

One base-arm wall-clock outlier (ld3 130 s wall vs ~17 s predicted) — queue/
tenant blip, `predicted_ts` normal; content unaffected.

### Content quality (owner-readable: `/tmp/opencode/lora/eval/{base,adapter}/*.txt`)

Consistent style shift on all 12 prompts:
- **Denser, code-first**: adapter answers open with the canonical
  implementation, then argue orderings/invariants from it. Base opens with
  prose framing.
- **More lines, fewer chars**: e.g. ld6 12→53 lines; sy3 18→43 lines; ~15%
  fewer characters overall. Structured headers/bullets, zero throat-clearing.
- **Spot-read verdicts**:
  - sy2-spsc (lock-free ordering): adapter arm *better* — correct
    relaxed/acquire/release assignment shown in code, crisp invariant
    statement, cache-line-separated indices. Base arm correct but slower to
    the point.
  - ld4-memory (ownership vs regions vs GC): adapter arm denser and further
    through the analysis in the same token budget, but conflates
    "Region Inference" with Rust NLL (borrow-checker lifetimes ≠ memory
    regions) — one observed conceptual slip. Base arm was cut off at the
    same point before reaching that section, so this is not a clean A/B on
    that claim.
- No degeneration, no `<think>` leakage (`--reasoning off` held), no repeat
  loops in either arm at 400 tokens.

## Interpretation

The adapter does what a reasoning-style distill should: restructures output
toward implementation-first, invariant-crisp answers. It does NOT add
language-design domain knowledge (none exists anywhere — expected). Whether
the style shift helps the owner's language-design work is an owner judgment
call on the 24 output files; the two spot-reads slightly favor the adapter
for systems content.

Cost: −17% decode today, recoverable to roughly −4..−8% (adapter matmul only)
by priming the draft context (`speculative.cpp` `ctx_dft` adapter wiring) —
small patch, separate stage.

## Decision options for owner

1. **Adopt for technical work** (per-request `"lora"` control or full-time) —
   needs launcher wiring (`scripts/vitriol` profile key + fingerprint `lora=`
   field) + ctx_dft patch to claw back speed.
2. **Keep on shelf, re-eval later** — file stays at
   `~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf`; loader/fork support
   committed; nothing in the live config changes.
3. **Drop** — delete the 467 MB file; keep the converter fix (upstream-grade
   bug fix regardless).

## Artifacts

- Inner commit `cd55f1f3a` — convert_lora_to_gguf.py v-reorder-on-LoRA fix
- `~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf` (f16 adapter, 467 MB)
- `/tmp/opencode/lora/` — adapter safetensors + venv + eval harness + raw
  outputs (`eval/{base,adapter}/{summary.json,*.txt,*.json}`)
- `/tmp/opencode/lora/server-adapter.log` — adapter-arm server log
- Unrelated uncommitted inner-repo state observed (in-source cmake artifacts
  untracked; `Makefile`, `tools/server/server-context.cpp` modified by other
  workstreams) — NOT touched by this stage.
