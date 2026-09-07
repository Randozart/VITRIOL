# Flash-Next Decode Optimization Roadmap (per-slot speed) — 2026-09-07

## Goal

Maximize **per-slot decode tokens/sec** for Qwen3.8-Flash-Next (qwen4exp, 176.9B,
Q2_K_XL, 73.4 GiB) on the Overlord-x8664 box (Arc B390 iGPU / NPU5 / 16c P-core /
62 GB LPDDR5X, ~85-90 GB/s). Not aggregate throughput — single-session latency.

Current measured state (2 slots × 131072 ctx, q4_0 KV, all-CPU `-ngl 0`,
`-ub 64 -t 8`):
- Single slot at 128k filled: ~10-11 t/s model (BW-bound ceiling ~12.9 t/s TQ3_0)
- 2 slots at 128k filled: ~5.5 t/s each (measured 5.69 / 5.69 t/s)
- Prompt eval (cold): ~4 t/s at 62 tokens
- Bottleneck: memory bandwidth (KV reads dominate at filled long context) +
  CPU MoE compute

## The five levers (in expected-impact order)

### Lever 1 — Speculative decoding with a small draft model (BIGGEST upside)

- **Source**: vLLM (EAGLE/Medusa/MTP), llama.cpp `--spec-draft-model`/`-md`.
- **Mechanism**: small model proposes N tokens, big model verifies in one forward;
  hides big-model per-token latency. Classic 1.5-3x on BW-bound MoE.
- **Current state**: BLOCKED for the MTP head (`mtp-Qwen3.8-Flash-Next-Q4_K_M.gguf`
  is a partial model; unsloth strips MTP layers from all quants — verified across
  every quant). BUT the generic draft path works with ANY small GGUF.
- **Need**: small draft GGUF (~0.5-1.5B, e.g. Qwen3-0.6B). ~1-2 GB disk (216 GB free).
- **Actions**:
  1. Download a small Qwen draft GGUF.
  2. Wire `-md <draft> --spec-draft-n-max 3` (+ `-td` threads, `-ctkd/ctvd` KV).
  3. Measure acceptance rate + realized t/s; tune n_max (1/3/5/8).
  4. If acceptance low, try a draft better matched to Qwen3.8 family
     (e.g. Qwen3-1.7B / Qwen3-4B-Instruct-Q4_K_M).
- **Risks**: low acceptance = wasted BW; draft adds KV + RAM. Mitigate by n_max sweep.

### Lever 2 — KV cache quantization TQ3_0 (free, in-tree, our own quant)

- **Source**: vLLM FP8 KV; llama.cpp TQ3_0/TQ3_1S = VITRIOL TurboQuant (3.5 bpw).
- **Current**: q4_0 KV both K+V.
- **Gain**: at filled 128k, KV read 2.6 → 2.0 GiB/forward ≈ 20% less KV BW
  ≈ +5-8% decode at long context.
- **Caveat**: `fattn rejects TQ3_*` note in llama-kv-cache.cpp is CUDA-specific;
  on this CPU/SYCL box K-side verified correct. V-side TQ3 on CPU needs a runtime
  FA check (the code comment flagged the fattn fallback path).
- **Actions**: `--cache-type-k tq3_0 --cache-type-v tq3_0`, verify FA active +
  no quality regression on a canned prompt; else keep V as q4_0, K as tq3_0.

### Lever 3 — Prefix caching / KV-shift reuse (free, ideal for OpenCode)

- **Source**: vLLM automatic prefix caching; llama.cpp `--cache-reuse N` +
  server `prompt_cache` (RAM token cache).
- **Current**: `n_cache_reuse = 0` default; prompt_cache exists but unused by us.
- **Gain**: repeated conversation prefixes skip prefill → prompt eval ~0 for
  cached turns. Big for interactive coding where system prompt + prior turns
  are re-sent each request.
- **Actions**:
  1. `--cache-reuse 256` (min chunk size for KV-shift reuse).
  2. Verify server `--prompt-cache` / prompt_cache path is active (RAM-cache).
  3. Measure a repeated-prefix request: prompt t/s should jump sharply.

### Lever 4 — Chunked prefill (verify + enable)

- **Source**: vLLM interleaves long prefill with decode; llama.cpp chunked prefill.
- **Gain**: slot 0's huge prefill won't starve slot 1's ongoing decode (2 concurrent
  users stay responsive).
- **Actions**: confirm chunking flag in this fork (search `--ctx-chunk` /
  chunked prefill in server), enable, verify 2-slot interleave.

### Lever 5 — VITRIOL-specific (already our innovation, keep as base)

- `--cpu-moe`, expert LRU + hot-profile preload (E31), streaming DMA, zero-copy
  mmap. These are AHEAD of vLLM/llama.cpp — keep.
- Ideas that were measured/considered and REJECTED:
  - **NPU as MoE router**: NPU ~80% iGPU speed, FP16-only, no BW advantage → dead.
  - **ESIMD/XMX**: targets compute; we're BW-bound, so polish after BW solved.
  - **KV pinning to on-die SRAM**: not available on PTL.
  - **PLE → IQ2_XS requant**: blocked — PLE is [160, 320M], ncols=160 not divisible
    by 256, so IQ/Q2/TQ all fall back to IQ4_NL (the 4.25bpw floor for 160-wide rows).

## Execution order

1. Write this roadmap (done).
2. Lever 2 (TQ3_0 KV) — one-line, lowest risk, immediate.
3. Lever 3 (cache-reuse + prompt cache) — two flags, immediate.
4. Lever 4 (chunked prefill) — verify flag surface.
5. Lever 1 (draft model) — download + wire + n_max sweep; biggest expected win.
6. Re-measure per-slot t/s after each; record in EXPERIMENT_LOG.md.

## Measurement protocol (per change)

- Server: 2 slots × 131072 ctx, `-ub 64 -t 8 -ngl 0`, alias flash-next.
- Warmup: 1× 128-tok generation to fault in hot expert pages.
- Measure: 3× separate /v1/chat/completions (fresh slots), 128-tok generations,
  report `predicted_per_second` median + server `print_timing` eval line.
- Track memory: `systemctl --user show vitriol-flashnext -p MemoryCurrent`.
- Compare against baseline 5.69 t/s (2 slots) / ~10-11 t/s (1 slot at 128k).

## Files / config touched

- `~/.config/systemd/user/vitriol-flashnext.service` — flags (Levers 2/3/4/1).
- `~/.config/opencode/opencode.jsonc` — unchanged unless draft changes model.
- `/home/prestopoverlord/models/` — new draft GGUF (Lever 1).
- `EXPERIMENT_LOG.md` — record each lever's measured delta.