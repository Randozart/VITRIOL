# KTransformers Analysis for VITRIOL

## Summary
KTransformers is a "Mirror Image" of VITRIOL - while KTransformers turns CPU into a co-processor, VITRIOL uses the CPU only for orchestration and does direct NVMe→GPU DMA.

---

## Key Files Analyzed

| File | Purpose |
|------|---------|
| `kt-kernel/operators/moe_kernel/moe.hpp` | C++ MoE kernel (800 lines) |
| `archive/ktransformers/server/backend/base.py` | Async server framework |
| `archive/ktransformers/optimize/optimize.py` | Layer placement optimizer |
| `optimize_rules/*.yaml` | YAML placement configs |

---

## Key Pattern 1: Arithmetic Intensity Guided Offloading

From their docs:
- **Attention (MLA)**: High arithmetic intensity (512) → GPU
- **MoE Experts**: Low intensity (0.075), massive data → CPU
- **Embeddings**: Accessed every token → GPU (high bandwidth)

**VITRIOL Application:**
- Keep **Attention layers** locked in VRAM (static)
- Stream **FFN/MLP** blocks via Moore Stream (fluid)
- Use 256MB BAR1 window for the streaming portion

---

## Key Pattern 2: Layer Placement YAML

From `DeepSeek-V2-Lite-Chat-gpu-cpu.yaml`:

```yaml
# GPU: layers 0-9
- match:
    name: "^model\\.layers\\.(0|[1-9])\\."
  replace:
    class: ktransformers.operators.experts.KTransformersExperts
    kwargs:
      prefill_device: "cuda:0"
      generate_device: "cpu"

# CPU: layers 10-29
- match:
    name: "^model\\.layers\\.([12][0-9])\\."
  replace:
    kwargs:
      prefill_device: "cpu"
      generate_device: "cpu"
```

**VITRIOL Application - Alembic Substrate Config:**
```yaml
# VITRIOL Layer Placement
static_layers: [0, 1, 2, 3, 4, 5]  # Attention, locked in VRAM
streaming_layers: [6, 7, 8, 9, ...]  # FFN, Moore Stream from SSD
transfer_threshold: 256  # MB - BAR1 window size
```

---

## Key Pattern 3: Expert Prefetch Logic

KTransformers uses async/await for overlapping:
- GPU: Attention computation
- CPU: Expert loading (in their case, math; in ours, DMA)

**VITRIOL Double-Buffer Logic:**
```
Layer N (GPU compute)  →  Layer N+1 (DMA stream in background)
    ↓                          ↓
Completion Fence   →   Completion Fence
```

---

## Key Pattern 4: Device Specification

Each operator has:
- `generate_device` - where to run during token generation
- `prefill_device` - where to run during prompt processing
- `generate_op` / `prefill_op` - which kernel to use
- `out_device` - where to move output

**VITRIOL Application:**
- `static_device: "gpu_vram"` - locked weights
- `streaming_device: "nvme_bar1"` - DMA from SSD
- `transfer_op: "vitriol_dma_flow"` - our custom op

---

## Architecture Comparison

| Aspect | KTransformers | VITRIOL |
|--------|---------------|---------|
| **Hot Path** | CPU math (AMX/AVX512) | GPU (CUDA) |
| **Cold Path** | GPU (minimal) | NVMe DMA |
| **CPU Role** | Compute (MoE experts) | Orchestration only |
| **Target** | Modern Xeon (400GB/s RAM) | Legacy Ivy Bridge |
| **Data Transfer** | RAM→VRAM (slow) | NVMe→VRAM (P2P DMA) |

---

## Key Code Locations

### MoE Kernel (C++ Template)
- Location: `/mnt/data/ai/KTransformers/kt-kernel/operators/moe_kernel/moe.hpp`
- Key class: `MOE_KERNEL_TP<T, bool PLAIN>`
- Contains weight loading from disk (lines 87-112)

### Async Backend
- Location: `/mnt/data/ai/KTransformers/archive/ktransformers/server/backend/base.py`
- Uses `async for` loops for streaming
- `ThreadContext` manages request lifecycle

### Layer Placement
- Location: `/mnt/data/ai/KTransformers/archive/ktransformers/optimize/optimize.py`
- Parses YAML and injects custom modules
- Uses regex for layer matching

---

## What to "Heist" for VITRIOL

### 1. YAML Placement Config
Create `alembic_substrate.yaml`:
```yaml
# Example for Qwen 3.5 9B (32 layers)
static:
  # Attention layers - always in VRAM
  - type: "attention"  
    layers: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    location: "gpu_vram"
    
streaming:
  # FFN layers - slide window from SSD
  - type: "ffn"
    layers: [12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31]
    location: "nvme_bar1"
    window_size: 256  # MB
    prefetch_ahead: 1  # layers
```

### 2. Double-Buffer C++ Logic
In llama.cpp, implement:
```cpp
// Pseudo-code for double-buffer DMA
void prefill_layer(int layer_id) {
    // Start DMA for next layer while current computes
    if (layer_id < max_layer - 1) {
        vitriol_async_dma(layer_id + 1);  // Non-blocking
    }
    // Wait for current layer weights
    vitriol_wait_fence(layer_id);
    // Compute
    cublas_matmul(layer_id);
}
```

### 3. Transfer Map
From their `transfer_map`:
```python
transfer_map = {10: "cpu"}  # Layer 10 triggers CPU offload
```
For VITRIOL: `{256: "remap_bar1"}` - when window fills, remap

---

## Conclusion

KTransformers teaches us:
1. **Separate by intensity** - High math → GPU, high data → CPU/SSD
2. **YAML-based placement** - Define what's static vs fluid
3. **Async overlap** - Prefetch next while computing current
4. **Transfer threshold** - When to move data between devices

**VITRIOL advantage**: On Ivy Bridge, CPU is too slow for MoE math, so we bypass it entirely with NVMe P2P DMA. KTransformers' async scheduling pattern is exactly what we need for the Moore Stream.

---
*Analysis complete. KTransformers repo available at `/mnt/data/ai/KTransformers/`*