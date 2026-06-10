# VITRIOL DMA Fix: Per-Slot Events + Buffer set_tensor Bug

**Date:** 2026-06-10 13:05-14:30  
**Author:** opencode  
**Status:** Fixed and verified

## Summary

Two bugs were found and fixed that prevented VITRIOL's `stream` mode from
working on Pascal (GTX 1070 Ti):

1. **Per-slot CUDA events** — prevents DMA corruption during LRU eviction
2. **Buffer `set_tensor`/`get_tensor` using wrong base pointer** — all tensors
   in a multi-tensor VITRIOL buffer overwrote each other at offset 0

## Bug 1: Buffer set_tensor/get_tensor uses ctx->base instead of tensor->data

### Root Cause

`vitriol-buffer.cpp:112-119` and `vitriol-vk-buffer.cpp:91-98` implemented
`set_tensor`/`get_tensor` using `ctx->base + offset`:

```cpp
memcpy((char *)ctx->base + offset, data, size);  // WRONG
```

The correct pattern (from `ggml-backend.cpp:2233` CPU buffer):

```cpp
memcpy((char *)tensor->data + offset, data, size);  // CORRECT
```

### Impact

When a VITRIOL buffer contained multiple tensors (e.g., 84 expert-weight
tensors across 28 layers × 3 MoE projections), every `ggml_backend_tensor_set`
call wrote to `ctx->base + 0`, overwriting the previous tensor's data. Only
the last-loaded tensor's data survived. All expert weight data was garbage
except the final tensor.

This made `VITRIOL_MODE=stream` produce immediate EOS on every prompt.
The model appeared to load fine (no errors) but responded with empty content.

### Files Fixed

| File | Lines | Change |
|------|-------|--------|
| `ggml/src/ggml-cuda/vitriol-buffer.cpp` | 112-119, 122-130 | `set_tensor`, `get_tensor`, `set_tensor_2d` |
| `ggml/src/ggml-vulkan/vitriol-vk-buffer.cpp` | 91-98 | `set_tensor`, `get_tensor` |

## Bug 2: Single Global CUDA Event for LRU Cache (Race Condition)

### Root Cause

A single `static CUevent g_lru_event` was shared across all LRU VRAM slots.
The synchronization flow was:

```
DMA writes Expert B to slot X → record g_lru_event on lru_stream
Compute reads Expert A from slot X → wait for g_lru_event on compute_stream
```

The global event couldn't distinguish "DMA for slot X is done" from "compute
for slot X is done". When slot X was evicted for a new expert, there was no
way to ensure the compute stream had finished reading slot X before the DMA
stream overwrote it.

On Pascal (CC 6.1), the copy engine and compute SMs are fully independent
with no automatic coherence, so the overwrite happened while compute was
still reading → corrupted expert weights.

### Fix: Per-Slot Events

Replaced `g_lru_event` with `g_lru_slot_events[]` — one CUevent per LRU slot.
The corrected flow:

```
1. DMA writes to slot S on lru_stream → record slot_events[S] on lru_stream
2. Compute reads slot S → wait for slot_events[S] → matmul → record slot_events[S] on compute_stream
3. Eviction overwrites slot S → wait for slot_events[S] on lru_stream → safe to DMA
```

### Files Changed

| File | Lines | Change |
|------|-------|--------|
| `vitriol-cuda-integration.h` | + | New `vitriol_lru_mark_compute_done()` declaration |
| `vitriol-cuda-integration.cpp` | 60-62, 717-723, 755-765, 793-846, 885-901, 917-942 | Per-slot events, bidirectional sync |
| `ggml-cuda.cu` | 2700, 2771-2773 | Call `mark_compute_done` after matmul |

## Verification

All tests pass with both VITRIOL_MODE=stream and VITRIOL disabled:

| Test | Without VITRIOL | With VITRIOL_MODE=stream |
|------|-----------------|--------------------------|
| "Say hello in one word." | "Hello" ✅ | "Hello" ✅ |
| "Say 1+1=" | "1 + 1 = 2" ✅ | "1 + 1 = 2" ✅ |
| "What is 2+2?" | "4" ✅ | "4" ✅ |

## Models Available for Benchmarking

| Model | File Size | Type | Active Params | Needs VITRIOL? |
|-------|-----------|------|---------------|----------------|
| Mellum2 12B MXFP4_MOE | 6.54 GB | MoE, MXFP4 | 2.5B | No (fits VRAM) |
| DeepSeek-Coder-V2-Lite IQ2_M | 5.9 GB | MoE, IQ2_M | ~2.5B | No (fits VRAM) |
| Gemma 4 26B Q4_0 | 14 GB | MoE, Q4_0 | 4B | Yes (14 GB > 8 GB) |

## Benchmark Results (2026-06-10)

Testing with "Write a Python function for merge sort." prompt, 64-token generation.

| Model | Config | Avg t/s | Context | Notes |
|-------|--------|---------|---------|-------|
| **DeepSeek-Coder-V2-Lite IQ2_M** | -ngl 28, -c 4096 | **~50 t/s** | 4K | 🏆 Fastest. 5× Mellum2. IQ2_M quality OK for code. |
| **Mellum2 12B MXFP4_MOE** | -ngl 28, -c 32768 | ~10 t/s | 32K | More context, better quantization, but 5× slower. |
| **Gemma 4 26B Q4_0** | -ngl 28, VITRIOL | ❌ OOM | — | Needs 11 GB CUDA_Host; system has 15 GB RAM, only 2.2 GB free. |

**DeepSeek-Coder-V2-Lite IQ2_M** is the clear winner for programming speed on this
hardware (GTX 1070 Ti, 8 GB VRAM, 15 GB RAM). Its ~50 t/s makes it usable for
interactive coding tasks. The tradeoff is limited context (4096 with -c flag).
Larger context (16384+) causes KV cache OOM on this GPU.

**Gemma 4 26B** cannot run on this system. The VITRIOL CUDA_Host buffer requires
~11 GB of page-locked RAM, but the system has only 2.2 GB free. Even with
swap, the OOM killer terminates the process. Would need a system with ≥24 GB
RAM or a different approach (e.g., disk offload).

### DeepSeek Specific Fix

The VITRIOL tensor pattern `tensor_name.find("exps")` was too broad — it matched
both `_exps.weight` and `_exps.scale` tensors. The `.scale` tensors are tiny
per-expert scale arrays that don't need VITRIOL DMA treatment. Fixed pattern to
`"exps.weight"` in `llama-model-loader.cpp:1255`.

## Known Issues (pre-existing, not introduced by this fix)

- **Lazy locking path** (`VITRIOL_MAX_LOCKED_MB`): When the page-lock budget
  is exhausted, `cuMemcpyHtoDAsync` receives a pageable source pointer and
  crashes. This affects the per-expert loop path. Workaround: use
  `VITRIOL_MODE=stream` without `VITRIOL_MAX_LOCKED_MB`.
- **VITRIOL_ENGINE_MODE** env var: Not a valid VITRIOL setting. The correct
  env var is `VITRIOL_MODE=stream`. AGENTS.md has been corrected.
