# VITRIOL Store Wire-in (flag-gated, non-destructive)

Date: 2026-09-23. Status: building.

## Principle

The mmap disk-offload path stays untouched and default. A new flag
`VITRIOL_STORE=1` + `VITRIOL_STORE_PATH=/dev/nvme0n1p3` selects the
store-backed cold source. Universal engine: same buffer abstraction, same LRU,
same ggml - only the byte source changes. Default off = today's exact behavior.

## Data flow (store-backed)

```
[ggml tensor] -> vitriol_buffer_set_tensor (records tensor_base + role,
                 NO data copy in store mode)
  -> vitriol_ensure_expert_locked(store_backed):
       if slice not filled: store_read_expert(st, tid, expert_idx, pinned_addr)
       mlock + cudaHostRegister (pinned RAM)
  -> ggml reads tensor_base (the pinned RAM), unchanged
```

mmap path: set_tensor copies `data` into mmap-backed pages; LRU mlock/pins.
Both paths present; flag selects. Byte-identical output gate.

## Changes

1. `vitriol-cuda-integration.h` (nvcc-parsed, C++20-safe):
   - `bool store_backed;` in `vitriol_config_t` (cheap, nvcc-safe).
   - NO char[] path in struct - host-only global in .cpp.

2. `vitriol-cuda-integration.cpp`:
   - `static char g_store_path[256]` + accessor `vitriol_store_path()`.
   - env parse: `VITRIOL_STORE=1` -> store_backed; `VITRIOL_STORE_PATH`.
   - tid map: at `vitriol_buffer_set_tensor`, record tensor_base->tid via
     `get_layer_index(tensor_base)` + role (ffn_up/gate/down/shexp) looked up
     against store_tensor_desc_at().

3. `vitriol-buffer.cpp`:
   - `vitriol_buffer_type_alloc_buffer`: if store_backed, open store, allocate
     pinned host RAM (existing stream-mode pattern: mmap anon + mlock +
     cudaHostRegister), NO GGUF mmap. Store handle in buffer context.
   - `vitriol_buffer_set_tensor`: store mode -> skip copy from `data`, record
     tid mapping. mmap mode -> unchanged.
   - free_buffer: close store.

4. LRU fill in `vitriol-cuda-integration.cpp` `vitriol_ensure_expert_locked`:
   - store_backed: before mlock, if slice not yet filled, store_read_expert.

## Acceptance

- Output-equivalence gate: load model, same seed, one decode; store-backed
  tokens == mmap-path tokens.
- Then 3x64 unit bench store vs mmap (toy-efficiency proof).

## Status

- 2026-09-23 wire-in code complete + committed (e4e8e1141, 0465a6489):
  - store_backed config + host-only store path
  - VITRIOL_STORE / VITRIOL_STORE_PATH env parse
  - buffer: store_open + pinned anon mmap; set_tensor skips copy for expert
    tensors only, registers tensor_base->tid by name
  - LRU: ensure_expert_locked fills slice from store_read_expert before mlock
  - CMake: integration.cpp + buffer.cpp to C++26
  - Fixed orphaned MTP arch files breaking the llama build (excluded from
    GLOB); store fill gate on store_backed (not lazy-lock budget)
- PENDING: output-equivalence gate. Blocked in-session by: 84G model slow
  load, live daily-driver llama-server on the GPUs (pid 80601), and
  -c 512 -ngl 99 VRAM overflow (compute buffers). Run via scripts/vitriol
  launcher with the certified geometry (small ctx, ub 64) after stopping the
  daily server.

## Config keys (flags too, per doctrine)

- `VITRIOL_STORE=1`
- `VITRIOL_STORE_PATH=/dev/nvme0n1p3`

## Commits

- config + env parse
- buffer store-backed alloc + set_tensor
- LRU fill + tid map
- equivalence gate