# VITRIOL - Spatial Transformer for Infinite VRAM
<img src="vitriol_logo.svg" alt="VITRIOL" width="200"/>

GPU memory streaming kernel module using Brief language.

## Overview

VITRIOL maps GPU memory from SSD to GPU via PCIe P2P DMA, enabling models larger than GPU VRAM by streaming layers on-demand.

## Quick Start

```bash
# Compile Brief to C kernel module
brief-compiler c vitriol.bv --target linux_kernel

# Build kernel module
make

# Load module
sudo insmod vitriol.ko

# Check dmesg
dmesg | tail

# Unload
sudo rmmod vitriol
```

## Hardware

- GPU: NVIDIA GTX 1070 Ti (`10de:1b82`)
- PCIe P2P DMA for zero-copy transfers

## Architecture

```
┌─────────────┐     PCIe      ┌─────────────┐
│     SSD     │◄────────────►│  GPU VRAM   │
└─────────────┘   P2P DMA    └─────────────┘
       │                              ▲
       │         VITRIOL              │
       ▼                              │
┌─────────────────────────────────────┐
│         Linux Kernel Module         │
│  - Layer discovery                  │
│  - Double-buffered DMA              │
│  - LRU eviction policy              │
└─────────────────────────────────────┘
```

## Files

- `vitriol.bv` - Brief source (main)
- `vitriol_new_ffi.bv` - New FFI syntax
- `vitriol.c` - Generated C code
- `vitriol.ko` - Compiled kernel module

## Requirements

- Linux kernel headers
- Brief compiler (`cargo build` from `../brief-compiler`)
- NVIDIA GPU with P2P support