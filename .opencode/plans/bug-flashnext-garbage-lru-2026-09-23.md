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

## ROOT CAUSE FOUND (2026-09-23, debug run)

**Not an offset bug. It's RLIMIT_MEMLOCK = 8192 KB (8 MB).**

- `ulimit -l` = 8192. `vitriol_ensure_expert_locked` mlock's each expert
  slice; cumulative >8MB -> `mlock slice(921600) failed: Cannot allocate
  memory` -> the lazy buffer pages are never locked/populated -> LRU reads
  garbage -> "////////".
- The `cudaHostUnregister failed: pointer does not correspond to a registered
  memory region` is a secondary symptom: mlock failed -> register skipped ->
  eviction unregisters an unregistered slice.
- Eager path fails separately: 56G host-RAM alloc on 32G RAM box.
- Flash-next (84G, 56G experts) fundamentally needs streaming + mlock beyond
  8MB. The 27B daily driver is resident (fits VRAM) so it never hit this.
- CAP_IPC_LOCK is NOT set on build/bin/llama-server (getcap empty).
  AGENTS.md: CUDA pinned allocs don't count against RLIMIT_MEMLOCK, but plain
  mlock DOES.

## BUG 2 (2026-09-23, after CAP fix): LRU eviction unregisters in-use slices

After CAP_IPC_LOCK fixed mlock, output became coherent ("Four." vs "////////")
BUT deterministic 32-token decode crashed:
`CUDA error: pointer does not correspond to a registered memory region`
at ggml-cuda.cu:219 during decode (task 6).

Root cause: LRU eviction in vitriol_ensure_expert_locked unregisters a slice
(cudaHostUnregister + munlock) when over the mlock budget, but a subsequent
DMA to that host pointer fails because it's no longer registered. The lazy
buft registers slices individually; eviction removes that registration while
the decode stream still references the page.

This is arm-A (mmap+LRU) specific. Store mode uses a different fill path.

## Fix direction

Either (a) do not unregister on eviction (leave registered, only munlock) -
cudaHostRegister has no memory cost beyond the pinning, and CUDA pinned
allocs don't count against RLIMIT_MEMLOCK per AGENTS.md; or (b) refcount
slices by in-flight stream events before unregistering.

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