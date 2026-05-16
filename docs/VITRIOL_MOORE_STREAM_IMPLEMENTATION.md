# VITRIOL Moore Stream Implementation Plan

## Status
- ✅ llama.cpp built with CUDA support
- ✅ Qwen 3.5 9B model loads and runs on GTX 1070 Ti (baseline working)
- ✅ NVIDIA GDS source cloned and analyzed
- ✅ Key DMA completion logic identified in nvfs-core.c
- ⏳ VITRIOL DMA integration pending

## Current Architecture (Baseline)

```
OpenCode → llama-server → ggml-cuda.cu → GPU VRAM (fully loaded)
                              ↓
                    [cudaMemcpyAsync at line 682]
```

## Target Architecture (VITRIOL Moore Stream)

```
OpenCode → llama-server → ggml-cuda.cu [MODIFIED] 
                              ↓
                    [VITRIOL DMA Hook]
                              ↓
              /mnt/data/ai/koboldcpp/ (SSD Model)
                              ↓
                    [DMA Direct to VRAM]
                              ↓
                         GPU VRAM (sliding window)
```

## Key Surgery Points (from ggml-cuda.cu)

Based on our earlier analysis:
1. **Line 682**: `cudaMemcpyAsync` - Primary tensor copy to GPU
2. **Line 2992**: Second `cudaMemcpyAsync` - Alternative copy path

## NVIDIA GDS Insights for Implementation

### 1. DMA Completion Mechanism (nvfs_io_complete)
NVIDIA uses Linux AIO with kiocb completion callbacks. Key structure:
- `nvfs_io_t` - Contains the I/O request and completion state
- Uses kernel callback pattern, not busy-polling

### 2. Memory Barrier Pattern
From the GDS source, we see the pattern:
- Use `wmb()` (write memory barrier) before triggering DMA
- Memory coherency is critical

### 3. The "Metapage" Concept
NVIDIA uses a shared memory page between kernel and userspace:
- Kernel writes fence value on completion
- Userspace polls the shared page (not kernel)

## Implementation Plan

### Phase 1: Identify Tensor Loading Points
1. Modify llama.cpp to track tensor file offsets
2. Intercept at `ggml_backend_cuda_buffer_set_tensor` (line 678)
3. Pass file offset to VITRIOL DMA module

### Phase 2: Create VITRIOL DMA FFI
```cpp
// In ggml-cuda.cu, replace cudaMemcpyAsync:
extern "C" {
    int vitriol_dma_transfer(uint64_t gpu_offset, uint64_t file_offset, size_t size);
    bool vitriol_check_completion();
}

// Instead of:
CUDA_CHECK(cudaMemcpyAsync(dst, src, size, cudaMemcpyHostToDevice, stream));
// Use:
if (vitriol_enabled) {
    vitriol_dma_transfer(gpu_addr, file_offset, size);
    // Wait for completion via metapage fence
    while (!vitriol_check_completion()) { }
}
```

### Phase 3: Sliding Window Logic
- BAR 1 on 1070 Ti is 256MB window
- Layers are ~150-400MB each
- Need to implement window remapping when layer > 256MB

### Phase 4: Integration with llama-server
- Keep model file on SSD (/mnt/data/ai/koboldcpp/)
- Load only first N layers to VRAM for "hot" state
- Stream subsequent layers on demand

## Next Steps for Today

1. **Find tensor file offsets in llama.cpp**
   - Examine `llama_model_loader` in src/llama-model-loader.cpp
   - Track which GGUF offset corresponds to each tensor

2. **Create VITRIOL FFI header**
   - Define C interface for DMA transfers
   - Create simple stub that falls back to cudaMemcpy

3. **Test with partial layer loading**
   - Load only first 10 layers to VRAM
   - Let rest fall back to CPU (baseline test)

4. **Then implement true DMA**
   - Use NVMe direct paths
   - Implement sliding window for large layers

## Technical Notes from GDS Analysis

From nvfs-core.c line 812-866:
```c
static void nvfs_io_complete(struct kiocb *kiocb, long res) {
    nvfs_io_t* nvfsio = container_of(kiocb, struct nvfs_io, common);
    nvfsio->ret = res;
    // Signal completion via shared memory
}
```

Key insight: Use Linux AIO for NVMe transfers, callback signals completion.

## Resource Summary

| Component | Location | Size |
|-----------|----------|------|
| Model | /mnt/data/ai/koboldcpp/ | 5.5GB |
| llama.cpp build | /mnt/data/ai/llama.cpp/bin/ | 9.5MB (llama-server) |
| CUDA lib | /mnt/data/ai/llama.cpp/bin/libggml-cuda.so | 74MB |
| GDS source | /mnt/data/ai/gds-nvidia-fs/src/ | 80KB |
| Swap | /mnt/data/ai/swap/ | 16GB |

---
*Implementation plan derived from NVIDIA GDS source analysis and llama.cpp CUDA backend surgery points identified during session.*