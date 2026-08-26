# AirLLM-Inspired Optimizations — Analysis & Plan

Reference: [AirLLM](https://github.com/lyogavin/airllm) — Gavin Li, 2023

## Architecture Comparison

| Dimension | VITRIOL | AirLLM |
|-----------|---------|--------|
| Target models | MoE only (sparse experts) | Dense only (sequential layers) |
| Core insight | 8/256 experts active per token → only swap experts | 1 layer at a time → never need all 80 layers in VRAM |
| Throughput | **6.21 tok/s** (real-time chat) | **~0.1-0.5 tok/s** (offline batch) |
| VRAM needed | 8 GB (GTX 1070 Ti) | 4 GB (any GPU) |
| Model size supported | 35B (MoE) | 405B (dense) |
| Bottleneck | PCIe DMA for expert fetch | Disk I/O for layer fetch |

---

## Optimization A: Block-Quantized PCIe Transfer (`--transfer-compress`)

### What

Apply a secondary block-wise quantization layer specifically for PCIe transfer.
Weights live in host RAM at native Q2_K_XL precision. Before `cuMemcpyHtoDAsync`
into the LRU VRAM pool, they are repacked into a tighter block format (2-bit).
A lightweight CUDA kernel decompresses them on arrival into the VRAM slot.
The matmul then reads decompressed weights at full precision.

### Why It Works

The bottleneck is PCIe bandwidth (~12 GB/s Gen 3 x16). Compressing expert weights
by 2× before transfer means the same bandwidth moves 2× the experts per second.
Decompression latency is hidden behind the existing `cuStreamWaitEvent` sync point.

```
Current:  [host RAM 2.6 bpw] → PCIe → [VRAM 2.6 bpw] → matmul
Proposed: [host RAM 2.6 bpw] → pack(2-bit) → PCIe → unpack → [VRAM 2.6 bpw] → matmul
```

### Expected Gain

| Expert Size | Current PCIe Time | Compressed PCIe Time | Decompress Time | Net Gain |
|-------------|------------------|---------------------|-----------------|----------|
| 50 MB | 4.2 ms | 2.1 ms | ~0.5 ms | **+25% tok/s** |

### Files to Modify

| File | Change |
|------|--------|
| `vitriol-transfer-compress.cuh` | **New.** Block-wise packing kernel (CUDA). Pack: find block min/max, quantize. Unpack: dequantize to float16. |
| `vitriol-cuda-integration.cpp` | In `lru_init_pool`: compress expert before `cuMemcpyHtoDAsync`. After copy completes (`cuStreamWaitEvent`), launch unpack kernel on compute stream. |
| `vitriol-cuda-integration.h` | Declare `vitriol_transfer_compress()`, `vitriol_transfer_decompress()`. Add config field. |
| `scripts/vitriol` | `--transfer-compress on\|off` flag, `VITRIOL_TRANSFER_COMPRESS` env var. |

### Implementation Sketch

```cpp
// vitriol-transfer-compress.cuh
// Block size: 256 elements (matching warp size for coalesced access)
// Each block stores: min_f16, max_f16, 256 × 2-bit values (64 bytes)
// Compression ratio: 16× (FP16) or ~2× (native fp16 vs 2-bit + metadata)

__global__ void decompress_block(
    const void* packed,  // input: block-min, block-max, 2-bit values
    half*       output,  // output: float16
    int         n_blocks
) {
    int block = blockIdx.x;
    // load min/max from packed header
    // dequantize: val = min + (code / 3.0f) * (max - min)
    // store to output
}
```

### Risk

- Decompression kernel might not finish before matmul starts → add a sync
- Block artifacts from 2-bit quantization might degrade model quality (mitigation: benchmark perplexity)
- Only applies to LRU cache *misses* — hits already have decompressed weights in VRAM

### Effort

2-3 sessions. Most time goes into CUDA kernel tuning (occupancy, shared memory usage).

---

## Optimization B: Disk Offload Fallback (`--disk-offload`)

### What

Replace anonymous `mmap` + `mlock` + `cudaHostRegister` with file-backed `mmap`
directly from the GGUF file on the NVMe SSD. Expert weights are demand-paged
from disk. The GPU reads them over PCIe, triggering page faults that the kernel
resolves via NVMe reads.

### Why It Works

VITRIOL currently requires ~10 GB of system RAM for the expert weight buffer.
Users with 8-16 GB of RAM can't run it. By mapping directly from the GGUF file
on disk, the expert buffer consumes zero DRAM — the OS pages it in from NVMe
on demand.

### Expected Performance

| Storage | Latency | Bandwidth | Estimated tok/s |
|---------|---------|-----------|----------------|
| DRAM (current) | ~100 ns | ~20 GB/s | 6.21 |
| NVMe SSD | ~3-10 µs | ~3-7 GB/s | **1-2 tok/s** |
| HDD | ~5-10 ms | ~100 MB/s | <0.1 tok/s (not supported) |

### Files to Modify

| File | Change |
|------|--------|
| `vitriol-buffer.cpp` | Add `--disk-offload` code path. Instead of anonymous `mmap` + `mlock` + `cudaHostRegister`, do file-backed `mmap` of the GGUF file. Skip `mlock` entirely. Skip `cudaHostRegister` (or use with constraints). |
| `vitriol-cuda-integration.cpp` | Read `VITRIOL_DISK_OFFLOAD` config. Adjust `vitriol_cuda_init()` to log the mode. |
| `scripts/vitriol` | `--disk-offload` flag, `VITRIOL_DISK_OFFLOAD` env var. |

### Implementation Sketch

```cpp
// In vitriol-buffer.cpp, new code path:
int fd = open(gguf_path, O_RDONLY);
void* base = mmap(NULL, expert_size, PROT_READ, MAP_SHARED, fd, expert_offset);
// NO mlock — pages come from disk
// NO cudaHostRegister — CUDA can still read from mmap'd memory on supported GPUs
//   (Pascal+ supports cudaHostRegister on file-backed mmap, but performance varies)
close(fd);
```

### Caveats

- **GPUDirect Storage (GDS)** would be faster but requires compatible hardware (NVIDIA
  Tesla/Quadro, specific NVMe drives) and kernel module setup. Not worth the complexity
  for a fallback path. Use plain `mmap` instead.
- **Pascal GPUs** (GTX 1070 Ti) support `cudaHostRegister` on file-backed `mmap`.
  Turing+ is faster. Maxwell and older may silently fall back to CPU copies.
- **Hot data**: the OS page cache will keep frequently-used expert pages in DRAM
  automatically. After a few minutes of inference, hot experts will be cached in
  RAM anyway — performance approaches RAM Shot levels for those experts.

### Effort

1 session. Mostly plumbing — the `mmap` infrastructure already exists.

---

## Optimization C (Deferred): Layer-Swap Engine for Dense Models

### What

Add an `--engine-mode layer-swap` fallback that loads one transformer layer at a
time into VRAM, processes it, evicts it, and loads the next. Directly copied from
AirLLM's architecture. This would make VITRIOL work with dense models (Llama 3.1
70B, DeepSeek V4, etc.) at the cost of real-time chat.

### Why Deferred

This is a fundamentally different engine — not an optimization of the existing one.
It requires:
1. A different tensor loading pipeline (layer-at-a-time instead of all-at-once)
2. A different graph scheduler (single-layer compute graphs)
3. A different memory management strategy (evict layer N-1 before loading layer N)

Estimated effort: 5-8 sessions. Worth doing only after the core MoE path is
feature-complete and well-tested.

### Expected Performance

| Model | GPU | Estimated tok/s | Use Case |
|-------|-----|----------------|----------|
| Llama 3.1 70B | 8 GB | ~0.3-0.5 | Offline batch processing |
| Llama 3.1 70B | 24 GB | ~1-2 | Interactive (slow) |
| DeepSeek V4 (dense) | 8 GB | ~0.2-0.4 | Offline batch |

---

## Summary

| # | Optimization | Effort | Gain | Dependency |
|---|-------------|--------|------|------------|
| A | Block-quantized PCIe transfer | 2-3 sessions | **+25% tok/s** | None |
| B | Disk offload fallback | 1 session | **Enables low-RAM machines** | `--disk-offload` flag |
| C | Layer-swap engine | 5-8 sessions | **Dense model support** | A + B + core stable |

---

*Last updated: 2026-05-17*
