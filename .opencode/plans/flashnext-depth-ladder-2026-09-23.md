# Flash-Next Full Depth Ladder: mmap baseline vs store-backed

Date: 2026-09-23. Status: running.

## Hardware (corrected)

2x RTX 3060 (GA106, Ampere, compute 8.6, 12 GiB each). NOT Pascal - the
"Pascal compute buffers" phrase in AGENTS.md is 1070-Ti-era and stale.

## Model under test

Qwen3.8-Flash-Next-UD-Q3_K_XL (3 shards, 84G). Store on /dev/nvme0n1p3:
336 expert tensors (52.2 GiB) + 26.8 GiB per_layer_token_embd, byte-verified
identical to GGUF source.

## Two arms

| arm | cold source | env |
|---|---|---|
| A baseline | page-cache mmap of GGUF (disk_offload branch) | VITRIOL_MODE=stream VITRIOL_DISK_OFFLOAD=1 VITRIOL_MODEL_PATH=<shard1> |
| B store | O_DIRECT off /dev/nvme0n1p3 | + VITRIOL_STORE=1 VITRIOL_STORE_PATH=/dev/nvme0n1p3 |

Shared: -ngl 99 -ts 12,12 --main-gpu 0 -ub 64 -fa on. LRU active in both.

## Phase 0 - Shallow equivalence gate

Stop daily server (frees GPUs). Both arms, same seed, -n 32. Assert token-
identical. Mismatch = store fill broken = STOP, debug, no ladder.

## Phase 1 - Full depth ladder (per depth-recert-2026-09-04 method)

Per arm, depths {13K, 26K, 36K}: exact-fill via /tokenize, prefill from
server timings, decode 3x64 at depth (rounds 2-3 cache_hit, sparse-KV),
VRAM dev0/dev1, watch ~23 KiB/token creep (flash-next documented wall).

## Deliverable

.opencode/plans/flashnext-depth-ladder-2026-09-23.md: per-arm t/s table at
each depth, equivalence PASS/FAIL, VRAM creep, store-vs-mmap attribution.

## Sequencing

Ladder ONLY if Phase 0 passes. mmap baseline pathologically slow at depth =
a finding (the store's reason to exist), noted not fought.

## BLOCKER found 2026-09-23 (arm A first launch)

Arm A failed to load: `ggml_backend_cuda_buffer_type_alloc_buffer: allocating
29677 MiB on device 0: cudaMalloc OOM`. Root cause: expert weights were NOT
routed to the VITRIOL buffer type - they went to normal CUDA buffers.

- vitriol_get_expert_buffer_type_dev() is declared but never CALLED by the
  loader for CUDA (dead code).
- Commit 5fb6a67a9 fixed this routing for SYCL (make_cpu_buft_list extra-buft
  discovery + supports_op MUL_MAT_ID-only). The CUDA path never got the same.
- Consequence: flash-next (84G, 56G experts) cannot load at all - experts
  demand ~30G CUDA buffer per GPU that does not exist.
- This is PRE-EXISTING, affects store AND mmap paths equally. Not a store bug.

Fix direction (separate commit): mirror 5fb6a67a9's make_cpu_buft_list logic
for the CUDA backend - discover VITRIOL buft as an extra buft paired with the
owning GPU, and have the CUDA supports_op claim MUL_MAT_ID-only for VITRIOL
src0. Then re-run Phase 0.