# VITRIOL Project Status - Session Summary

## What We Accomplished

### 1. Environment Setup
- ✅ Cloned NVIDIA GDS source to `/mnt/data/ai/gds-nvidia-fs/`
- ✅ Built llama.cpp with CUDA support
- ✅ Model (Qwen 3.5 9B Q4_K_M) ready at `/mnt/data/ai/koboldcpp/`

### 2. Baseline Testing Results

| GPU Layers | GPU Memory | CPU Memory | Performance |
|------------|------------|------------|-------------|
| 25 (full)  | 3974 MiB   | 546 MiB    | **10.6 tok/s** |
| 15 (partial) | 2675 MiB  | 2385 MiB   | 5.3 tok/s   |

**Key Finding**: Full GPU offload (25 layers) gives ~2x performance over partial offload.

### 3. NVIDIA GDS Analysis
- Studied `nvfs-core.c` for DMA completion mechanism
- Identified `nvfs_io_complete` callback pattern (line 812)
- Found metapage/shared memory signaling for completion
- Understood memory barrier (`wmb()`) usage before DMA

### 4. Implementation Files Created

| File | Purpose |
|------|---------|
| `/mnt/data/ai/llama.cpp/include/vitriol-dma.h` | FFI header for DMA calls |
| `libvitriol/vitriol_layer_manager.py` | Dynamic layer management |
| `launch_vitriol_v2.sh` | Optimized launch script |
| `VITRIOL_MOORE_STREAM_IMPLEMENTATION.md` | Implementation plan |
| `NVIDIA_OPENSOURCE_TREASURES.md` | NVIDIA repos documentation |
| `NVIDIA_GDS_INFO.md` | GDS insights |

### 5. Surgery Points Identified

**ggml-cuda.cu**:
- Line 682: `cudaMemcpyAsync` - Primary tensor copy (HOST→DEVICE)
- Line 690: `cudaMemcpyAsync` - Tensor read (DEVICE→HOST)
- Line 2992: Alternative tensor copy path

## Current Architecture

```
OpenCode → llama-server (port 8279)
                 ↓
          ggml-cuda.cu (line 682)
                 ↓
           cudaMemcpyAsync
                 ↓
           GPU VRAM (25 layers = 3974 MiB)
                 ↑
Model: /mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf (5.5GB on SSD)
```

## Path to True NVMe DMA (Moore Stream)

### Phase 1: Current State ✅
- Full model on SSD
- 25 layers loaded to GPU
- Works but uses 5GB VRAM

### Phase 2: Smart Layer Loading (Next)
- Load only "hot" layers to GPU (e.g., first 15)
- Keep rest on SSD
- Use pread() to load from SSD on-demand
- Trade-off: ~50% performance but works with smaller VRAM

### Phase 3: True DMA (The Vision)
- Use NVIDIA GDS pattern
- Map GPU BAR1 aperture (256MB window)
- NVMe direct transfer to GPU VRAM
- Sliding window for layers > 256MB
- Requires: Kernel module + GPU ID spoofing

## Technical Notes from GDS

### DMA Completion Pattern (nvfs-core.c:812)
```c
static void nvfs_io_complete(struct kiocb *kiocb, long res) {
    nvfs_io_t* nvfsio = container_of(kiocb, struct nvfs_io, common);
    nvfsio->ret = res;
    // Signal completion via shared memory (metapage)
}
```

### Memory Barrier Pattern
```c
wmb();  // Write memory barrier before DMA
trigger_nvme_dma();
```

### The "Metapage" Concept
- 4KB shared page between kernel and userspace
- Kernel writes fence value on completion
- Userspace polls for fence change (fastest path)

## What's Needed for True DMA

1. **Kernel Module** - Map GPU BAR1 into physical address space
2. **Device ID Spoof** - Make GTX 1070 Ti appear as Tesla (patch GDS driver)
3. **Sliding Window** - Handle layers > 256MB (BAR1 limit)
4. **Completion Handler** - Implement metapage-based signaling

## Quick Start

```bash
# Launch VITRIOL stack
cd /home/randozart/Desktop/Projects/linux-pipe-module
chmod +x launch_vitriol_v2.sh
./launch_vitriol_v2.sh

# Test inference
curl http://localhost:8279/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Hello"}],"max_tokens":20}'
```

## Files Summary

| Path | Size | Description |
|------|------|-------------|
| `/mnt/data/ai/llama.cpp/bin/llama-server` | 9.5MB | CUDA inference server |
| `/mnt/data/ai/llama.cpp/bin/libggml-cuda.so` | 74MB | CUDA backend |
| `/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf` | 5.5GB | Model file |
| `/mnt/data/ai/gds-nvidia-fs/src/nvfs-core.c` | 81KB | GDS DMA source |

---
*Session complete. Baseline working at ~10.6 tok/s with full GPU offload.*