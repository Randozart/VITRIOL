# Alka Commands: Production Readiness Audit — 2026-05-13

> Analysis of which Alka instructions must be production-ready before VITRIOL can use Alka for the expert streaming DMA path.

Based on the investigation of llama.cpp's MoE expert loading code path (`ggml-cuda.cu:682`, `ggml-cuda.cu:2992`, `ggml-backend.cpp:1576-1660`), here is the exact instruction set required.

---

## Tier 1: CRITICAL — DMA Pipeline Core

These instructions form the hot path. Every inference token executes this sequence.
Must be hardware-tested and verified.

### FLOW (0x03) — NVMe→GPU Expert Transfer

**What it does:** Initiates a DMA transfer of expert weight data from NVMe SSD to GPU VRAM.

**Where it fits in llama.cpp:**
Replaces `cudaMemcpyAsync` at `ggml-cuda.cu:2992` (`ggml_backend_cuda_set_tensor_async`).
Also replaces `ggml_backend_cuda_buffer_set_tensor` at `ggml-cuda.cu:682` (for load-time transfer).

**Production requirements:**
- Must handle variable-sized transfers (expert tensors are 150-400MB each)
- Must correctly handle GGUF tensor offsets from the model file
- Must support multiple concurrent FLOWs for double-buffering
- Must return immediately (async), with FENCE for completion
- Target bandwidth: 6-12 GB/s (PCIe 3.0 x16)

**Current status:** Tool exists in Alka compiler (`tools/core/flow.zig`). Compiles to Metrod packet. Mock executor simulates transfer with memset. **No real DMA backend.**

### CLAIM (0x01) — Hardware Node Claim

**What it does:** Claims a PCIe device (GPU or NVMe), unbinds the kernel driver, maps BAR regions into userspace.

**Where it fits:**
Prerequisite for FLOW. Without CLAIM, there's no BAR access for DMA.

**Production requirements:**
- Must correctly identify device by PCI ID (vendor:device)
- Must unbind `nvidia` or `nvme` driver safely
- Must map BAR0 (control plane) and BAR1 (data plane) into process address space
- Must restore driver binding on exit (Azoth rollback)

**Current status:** Tool exists. PCI scanner works with `--probe`. Mock executor records claimed state. **No real driver unbind.**

### FENCE (0x05) — DMA Completion Poll

**What it does:** Spins on a GPU metapage register until it reaches an expected value, confirming prior DMA transfer completed.

**Where it fits:**
In the current cudaMemcpyAsync path, synchronization happens via `cudaStreamSynchronize`. FENCE replaces this — instead of blocking on a CUDA stream, it polls a hardware completion flag.

**Production requirements:**
- Must correctly poll the GPU's DMA completion metapage
- Must have a timeout (default 5s) with error return on timeout
- Must handle the case where DMA fails silently (metapage never reaches expected value)

**Current status:** Tool exists. Mock executor always passes. **No real metapage polling.**

### SHIFT (0x04) — BAR1 Sliding Window Remap

**What it does:** Remaps the 256MB BAR1 aperture window to a different physical offset. Required because any single DMA transfer larger than 256MB must be chunked.

**Where it fits:**
The 256MB BAR1 limit applies to all GTX 1070 Ti transfers on Ivy Bridge/Z77. SHIFT is called between FLOW chunks.

**Production requirements:**
- Must write to the BAR1 sliding window control register (typically at BAR0 + 0x4000)
- Must FENCE after each SHIFT before starting next FLOW
- Must correctly calculate the window base address

**Current status:** Tool exists. Mock executor records offset. **No real BAR register write.**

### SIGNAL (0x09) — Trigger GPU Compute

**What it does:** Triggers GPU to start computing with the newly loaded expert data. Equivalent to launching a CUDA kernel after a data dependency is satisfied.

**Where it fits:**
After FLOW+FENCE loads experts, SIGNAL tells the GPU to proceed with the MoE matmul.

**Production requirements:**
- Must be lightweight (just a register write or doorbell)
- Must not return until the GPU acknowledges the signal

**Current status:** Tool exists. Mock executor records signal vector. **No real GPU compute trigger.**

---

## Tier 2: REQUIRED — Safety & Orchestration

These instructions aren't in the hot path but are required for correct and safe operation.

### LIMIT (0x0E) — Thermal Hard Contract

**What it does:** Sets a thermal limit. If exceeded, the system hard-aborts.

**Production requirements:**
- Must work before any DMA (set limit, then begin transfers)
- Must be enforceable (sensor-based, not just a suggestion)

**Current status:** Tool exists. Mock executor sets halt_at/throttle_at. **No real sensor enforcement.**

### SENSE (0x07) — Read Temperature Sensor

**What it does:** Reads GPU temperature via nvidia-smi or sysfs.

**Production requirements:**
- Must complete in <1ms (called between tokens)
- Must not fail silently (return error if sensor unavailable)

**Current status:** Tool exists. Mock executor simulates temperature ramp.

### SYNC (0x06) — Memory Barrier

**What it does:** Ensures all prior memory operations (DMA writes, register writes) are visible before proceeding.

**Production requirements:**
- L3 (full device barrier) required between FLOW→SIGNAL
- Must use `__threadfence()` or equivalent GPU barrier

**Current status:** Tool exists. Mock executor counts cycles.

---

## Tier 3: IMPORTANT — Performance

### REFRACT (0x3B) — Sub-Tensor Slicing

**What it does:** Automatically slices a tensor larger than the BAR1 window (256MB) into smaller chunks with SHIFT/FLOW/FENCE triplets injected by the compiler.

**Production requirements:**
- Must read Vial's `MAX_WINDOW` parameter (256MB)
- Must auto-calculate slice count and offsets
- Must inject SHIFT/FLOW/FENCE for each slice

**Current status:** Tool exists. Mock executor reports slice count.

### PIPE (0x3C) — Continuous DMA Ring Buffer

**What it does:** Establishes a hardware ring buffer that streams data autonomously (NVMe→GPU) without CPU involvement after initiation. Used for double-buffering.

**Production requirements:**
- Must work with both NVMe and GPU as endpoints
- Must support 256MB ring size (ALKA_INTEGRATION.md spec)
- Must handle wrap-around correctly

**Current status:** Tool exists (opcode defined, mock executor simulates). **Not implemented in kernel module.**

---

## Tier 4: DESIRED — Recovery

### SNAP (0x0C) / REVERT (0x0D)

**What they do:** State serialization and restoration. Used for rollback on failure.

**Production requirements:**
- SNAP must capture all GPU registers that FLOW/FENCE modify
- REVERT must restore exact pre-DMA state
- Used in the Azoth rollback binary

**Current status:** Tools exist in mock executor.

---

## Implementation Priority

```
Iteration 1: CLAIM + FLOW + FENCE + SHIFT + SIGNAL (hot path)
Iteration 2: LIMIT + SENSE + SYNC (safety)
Iteration 3: REFRACT + PIPE (performance)
Iteration 4: SNAP + REVERT (recovery)
```

---

## Current Code Locations

| Component | Repository | Path |
|-----------|------------|------|
| Tool implementations | alka-lang | `src/tools/core/{claim,flow,shift,fence,signal}.zig` |
| Metrod codegen | alka-lang | `src/codegen/alka_bin.zig` |
| Mock executor | alka-lang | `src/executor/mock_executor.zig` |
| Kernel module (Athanor) | alka-lang | `src/athanor/vitriol_alka_*.c` |
| Compiled bin | alka-lang | `zig-out/bin/alka` |
| VITRIOL recipes | VITRIOL | `alka/recipes/*.alka` |
| VITRIOL vials | VITRIOL | `alka/vials/*.alkavl` |
