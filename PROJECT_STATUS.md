# VITRIOL Project Status

**Last Updated:** 2026-05-10  
**Goal:** SSD-to-GPU direct streaming for LLM inference on GTX 1070 Ti

---

## Hardware Configuration

| Component | Details |
|-----------|---------|
| GPU | GTX 1070 Ti (device 1b82, 8GB VRAM) |
| CPU | i7-3770 (Ivy Bridge, no AVX2) |
| Storage | NVMe SSD on /mnt/data/ai |
| VRAM Window | BAR 1: 256MB aperture |

---

## Current Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Project Files                            │
├─────────────────────────────────────────────────────────────┤
│ /mnt/data/ai/koboldcpp/          - Qwen 3.5 9B model (5.5GB)│
│ /mnt/data/ai/llama.cpp/bin/      - llama-server + CUDA libs │
│ /mnt/data/ai/gds-nvidia-fs/src/  - NVIDIA GDS source        │
│ /mnt/data/ai/swap/               - 16GB swap                │
├─────────────────────────────────────────────────────────────┤
│ vitriol-daemon/vitriol.ko        - Kernel module (410KB)   │
│ vitriol-daemon/vitriol-util      - Userspace utility (16KB)│
│ vitriol_new_ffi.bv              - Brief kernel spec        │
└─────────────────────────────────────────────────────────────┘
```

---

## What's Working

### 1. llama.cpp with CUDA ✅
- Built with CUDA support
- Qwen 3.5 9B loads and runs
- **Performance:** 10.6 tok/s with 25 GPU layers
- **GPU Memory:** 3974 MiB (model) + 192 MiB (KV) + 565 MiB (compute)

```bash
cd /mnt/data/ai/llama.cpp
./bin/llama-server -m /mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf -c 8192 -ngl 25 --port 5002
```

### 2. NVIDIA GDS Source Analyzed ✅
- Cloned from `github.com/NVIDIA/gds-nvidia-fs`
- Studied: `nvfs-core.c`, `nvfs-pci.c`, `nvfs-dma.c`
- Key findings: kiocb completion callbacks, metapage signaling, memory barriers

### 3. Kernel Module Built ✅
- **Location:** `vitriol-daemon/vitriol.ko`
- **Features:**
  - PCI probe for GTX 1070 Ti (10de:1b82)
  - BAR 0 mapping (16MB - Control Plane)
  - BAR 1 mapping (256MB - Data Plane)
  - DMA buffer allocation
  - Character device (/dev/vitriol)
  - IOCTL interface

---

## Key Files Created

| File | Purpose |
|------|---------|
| `vitriol-daemon/vitriol.c` | Kernel module source (11KB) |
| `vitriol-daemon/vitriol.ko` | Compiled module (410KB) |
| `vitriol-daemon/vitriol-util.c` | Userspace utility |
| `vitriol-daemon/vitriol-util` | Compiled utility |
| `/mnt/data/ai/llama.cpp/include/vitriol-dma.h` | DMA FFI header |
| `NVIDIA_GDS_INFO.md` | GDS analysis notes |
| `NVIDIA_OPENSOURCE_TREASURES.md` | NVIDIA repos documentation |
| `VITRIOL_MOORE_STREAM_IMPLEMENTATION.md` | Implementation plan |

---

## Next Steps

### Immediate (Requires sudo on target)
```bash
# Load kernel module
sudo insmod vitriol-daemon/vitriol.ko
dmesg | tail

# Test utility
sudo ./vitriol-daemon/vitriol-util status
sudo ./vitriol-daemon/vitriol-util bar1
```

### Integration with llama.cpp
- Replace `cudaMemcpyAsync` in `ggml-cuda.cu` (line 682)
- Use VITRIOL DMA for direct NVMe→GPU transfers
- Implement sliding window for layers > 256MB

### Safe Testing Options
1. **VM with GPU passthrough** - Safest, isolated environment
2. **Secondary cheap GPU** - GTX 750/Ti for testing
3. **USB GPU box** - Complete isolation via external enclosure

---

## Technical Notes

### From NVIDIA GDS (nvfs-core.c)
- Use `kiocb` completion callbacks for NVMe completion
- Shared "metapage" (4KB) for fast completion signaling
- `wmb()` memory barrier before triggering DMA

### From llama.cpp Surgery Points
- `ggml-cuda.cu:682` - Primary tensor copy (cudaMemcpyAsync)
- `ggml-cuda.cu:2992` - Alternative copy path

### BAR Configuration
- **BAR 0**: 16MB - Control plane, read-only GPU registers
- **BAR 1**: 256MB - Data plane, VRAM window for DMA

---

## Reference Commands

```bash
# Build kernel module
cd vitriol-daemon && make

# Load/unload module
sudo insmod vitriol.ko
sudo rmmod vitriol

# Check module info
modinfo vitriol.ko

# Test userspace utility
sudo ./vitriol-util status
sudo ./vitriol-util bar1

# Run llama-server
cd /mnt/data/ai/llama.cpp
./bin/llama-server -m /mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf -ngl 25 --port 5002
```

---

## Project Timeline

- **Session 1-2**: Hardware analysis, repo structure, VITRIOL shim
- **Session 3**: NVIDIA GDS discovery, Brief kernel spec
- **Session 4**: llama.cpp build with CUDA, baseline working
- **Session 5**: Kernel module implementation, compiled successfully

---
*Status: Kernel module ready. Testing requires target system with GPU.*