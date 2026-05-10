# VITRIOL Kernel Module - Implementation Complete

## What Was Built Today

### 1. Linux Kernel Module (`vitriol.c`)
- **Location**: `/home/randozart/Desktop/Projects/linux-pipe-module/vitriol-daemon/vitriol.c`
- **Size**: 410KB compiled
- **Features**:
  - PCI probe for GTX 1070 Ti (device ID 1b82)
  - BAR 0 mapping (16MB - Control Plane)
  - BAR 1 mapping (256MB - Data Plane/VRAM Window)
  - DMA buffer allocation
  - Character device interface (/dev/vitriol)
  - IOCTL interface for userspace control

### 2. Userspace Utility (`vitriol-util.c`)
- **Location**: `/home/randozart/Desktop/Projects/linux-pipe-module/vitriol-daemon/vitriol-util.c`
- **Size**: 16KB compiled
- **Commands**:
  - `status` - Check if module is loaded and GPU mapped
  - `bar1` - Get BAR1 VRAM window address
  - `transfer` - Test DMA transfer

### 3. Makefile
- Standard kernel module build system
- Commands: `make`, `make load`, `make unload`, `make clean`

## Architecture Pattern (Based on NVIDIA GDS)

```
┌─────────────────────────────────────────────────────────┐
│                  VITRIOL Kernel Module                  │
├─────────────────────────────────────────────────────────┤
│  PCI Probe (find GPU 10de:1b82)                         │
│       ↓                                                 │
│  BAR 0: Control Plane (16MB) ← Read GPU registers      │
│  BAR 1: Data Plane (256MB) ← DMA target window          │
│       ↓                                                 │
│  DMA Buffer: 1MB (allocatable)                          │
│       ↓                                                 │
│  Character Device: /dev/vitriol                         │
└─────────────────────────────────────────────────────────┘
                          ↓ IOCTL
┌─────────────────────────────────────────────────────────┐
│              Userspace (llama.cpp integration)          │
│  - Get BAR1 address                                     │
│  - Queue DMA transfers                                  │
│  - Poll for completion                                  │
└─────────────────────────────────────────────────────────┘
```

## Key NVIDIA GDS Patterns Implemented

1. **PCI Device Discovery**: Find GPU by vendor/device ID
2. **BAR Mapping**: Separate control (BAR0) and data (BAR1) planes
3. **DMA Buffer**: Coherent allocation for NVMe transfers
4. **IOCTL Interface**: Userspace/kernel communication
5. **Memory Barriers**: Prepared for wmb() before DMA triggers

## Next Steps (On Target System - Ivy Bridge PC)

### To Load and Test:
```bash
# On target system with GTX 1070 Ti

# 1. Load kernel module (requires root)
sudo insmod vitriol.ko
dmesg | tail

# Should see:
# VITRIOL: Initializing NVMe-to-GPU DMA module
# VITRIOL: Target: GTX 1070 Ti (device 1b82)
# VITRIOL: GPU configured successfully
# VITRIOL: Device node: /dev/vitriol

# 2. Check status
sudo ./vitriol-util status

# 3. Get BAR1 address
sudo ./vitriol-util bar1
```

### To Integrate with llama.cpp:
The kernel module provides:
- BAR1 address for mapping GPU VRAM window
- DMA transfer capability (via IOCTL)
- Memory-mapped I/O for GPU registers

Integration point: Replace `cudaMemcpyAsync` in `ggml-cuda.cu` with VITRIOL DMA calls.

## File Summary

| File | Size | Purpose |
|------|------|---------|
| `vitriol.ko` | 410KB | Linux kernel module |
| `vitriol-util` | 16KB | Userspace control utility |
| `vitriol.c` | 11KB | Kernel module source |
| `vitriol-util.c` | 4KB | Utility source |
| `Makefile` | 0.5KB | Build system |

## Important Notes

1. **This system is NOT the target** - The kernel module won't find the GPU (no GTX 1070 Ti)
2. **Compile on target system** - Kernel modules must match the running kernel
3. **Requires root** - Both module loading and utility need sudo
4. **GPU ID spoofing** - To use NVIDIA GDS, need to patch device ID check

---
*Kernel module ready. Transfer to target Ivy Bridge system for actual testing.*