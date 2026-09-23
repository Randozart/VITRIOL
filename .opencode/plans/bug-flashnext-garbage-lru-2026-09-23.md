# BUG: flash-next garbage output (////////) with LRU streaming active

Date: 2026-09-23. Status: open, debugging.

## Symptom

Qwen3.8-Flash-Next-UD-Q3_K_XL (84G, qwen4exp arch) on 2x RTX 3060:
- `VITRIOL_MODE=stream VITRIOL_MAX_LOCKED_MB=8192` (lazy buft, LRU active):
  model loads, serves, but generates `////////` (8+ identical slashes).
  Expert data reaching the GPU is wrong.
- `VITRIOL_MODE=stream` + `max_locked_mb` unset (eager buft):
  `alloc_tensor_range: failed to allocate CUDA_Host buffer of size
  55833792000` - 56G host RAM alloc, box has 32G. Eager path unusable for
  a 56G-expert model.

## What works

- Expert -> VITRIOL-buft routing (commit d7b722ea6): without it flash-next
  cannot load at all (56G experts -> cudaMalloc 29.7G OOM per GPU).
  Control confirmed: VITRIOL_MODE unset = load fails.
- Store on /dev/nvme0n1p3 byte-identical (336 tensors + embed).

## Root-cause hypotheses

1. LRU prefetch / vitriol_lru_prefetch_async reads expert_data from the lazy
   VITRIOL buffer into a VRAM pool. Wrong offset for heterogeneous per-tensor
   expert slices (per-tensor-kind layout) -> garbage.
2. Eager path's 56G host-RAM alloc is a sizing bug (should mmap GGUF via
   disk_offload, not alloc anon RAM).

## Repro

```
# arm A, LRU active -> garbage
export VITRIOL_MODE=stream VITRIOL_MAX_LOCKED_MB=8192
export VITRIOL_DISK_OFFLOAD=1 VITRIOL_MODEL_PATH=<shard1>
llama-server -m <shard1> -ngl 99 -ts 12,12 --main-gpu 0 -c 512 -ub 64 -fa on
curl /completion -d '{"prompt":"What is 2+2?","n_predict":16}'
# -> "//////////////"
```

## Debug plan

1. Verify set_tensor wrote correct data into the lazy buffer (offset/stride).
2. Trace vitriol_lru_prefetch_async for a known (tensor, expert) against the
   GGUF source bytes.
3. Check whether the 27B (working) and flash-next differ in expert tensor
   layout handled by the LRU.
4. Fix LRU offset bug OR eager-path sizing; re-verify output coherence.