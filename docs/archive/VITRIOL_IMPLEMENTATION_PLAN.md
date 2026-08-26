# VITRIOL Implementation Plan

**Version:** 2.0  
**Last Updated:** 2026-05-09  
**Hardware Target:** GTX 1070 Ti (8GB) + i7-3770 (Ivy Bridge)  
**Status:** Phase 1 Ready

---

## Executive Summary

VITRIOL is a **Direct NVMe-to-GPU streaming stack** designed to enable large LLM inference on legacy hardware with limited VRAM. The project consists of three layers:

1. **Context Rectifier (Python Shim)** - Filters bloated prompts before they reach the GPU
2. **Hardware Sentinel (Brief Kernel Module)** - Monitors thermal, aperture, and DMA safety
3. **Mercurial Bridge (DMA Engine)** - Streams model layers directly from NVMe to GPU VRAM

---

## Hardware Analysis Report

### System Configuration

| Component | Specification | Status |
|-----------|--------------|--------|
| **CPU** | Intel i7-3770 (Ivy Bridge) | AVX-only (no AVX2) |
| **GPU** | NVIDIA GTX 1070 Ti (8GB) | Device ID: `10de:1b82` |
| **Chipset** | Intel Z77 | Legacy PCIe aperture |
| **IOMMU** | Intel VT-d | **Enabled** (`iommu=pt`) |

### PCIe BAR Configuration

```
01:00.0 VGA compatible controller: NVIDIA Corporation GP104 [GeForce GTX 1070 Ti]

Memory at f6000000 (32-bit, non-prefetchable) [size=16M]     <- BAR 0
Memory at e0000000 (64-bit, prefetchable) [size=256M]        <- BAR 1 (VRAM WINDOW)
Memory at f0000000 (64-bit, prefetchable) [size=32M]         <- BAR 2
```

**Critical Finding:** The 256MB aperture on BAR 1 is the **only** path to VRAM. This requires a **sliding window** implementation for layers larger than 256MB.

### Thermal Sensors

```
/sys/class/hwmon/hwmon0: acpitz     (motherboard)
/sys/class/hwmon/hwmon1: coretemp   (CPU)
nvidia: NOT EXPOSED via hwmon
```

**Solution:** Thermal monitoring via `nvidia-smi` polling from userspace (Python shim).

---

## The Four Guardrails

### Guardrail 1: Address Translation Safety

**Risk:** DMA to wrong physical address corrupts system RAM  
**Protection:** Use `dma_map_single()` and `virt_to_phys()` - never raw pointers  
**Brief Contract:** `dma_mapped == true` precondition on all DMA transactions

### Guardrail 2: 256MB Aperture Limit

**Risk:** Mapping >256MB causes PCIe bus error and system latch-up  
**Protection:** Sliding window implementation with 256MB chunks  
**Brief Contract:** `sliding_window_required == true` triggers chunked streaming

### Guardrail 3: IOMMU Passthrough

**Risk:** DMA without IOMMU can corrupt any system memory  
**Protection:** `intel_iommu=on iommu=pt` kernel parameter  
**Brief Contract:** IOMMU validation transaction before first DMA

### Guardrail 4: Thermal & Power

**Risk:** Sustained LLM inference overheats aging GPU  
**Protection:** `nvidia-smi` polling, 85°C hard halt  
**Brief Contract:** `gpu_temp < MAX_TEMP` on all stream transactions

---

## Implementation Phases

### Phase 1: Context Rectification (Python Shim)

**Status:** Ready to Test  
**Risk:** None (userspace only)  
**Goal:** Solve immediate 389k character OOM crashes

#### Components

| File | Purpose | Port |
|------|---------|------|
| `vitriol_shim.py` | Context filtering proxy | 5005 |
| `test_shim.py` | Integration test suite | - |
| `safe_test_vitriol.sh` | Graduated testing script | - |

#### Features

- **Calcination (Truncation):** Keep system + last 4 messages only
- **Sublimation (Metadata Stripping):** Remove `<reasoning>`, `tool_results`
- **Coagulation (Formatting):** Clean ChatML for Qwen 3.5
- **Thermal Polling:** `nvidia-smi` check before forwarding requests
- **Hard Cap:** 7,000 tokens max (leaves 1,192 for generation)

#### Testing Procedure

```bash
# Terminal 1: Start KoboldCPP
cd ~/Downloads/koboldCPP
./run_qwen.sh

# Terminal 2: Test VITRIOL shim
cd ~/Desktop/Projects/linux-pipe-module
python3 test_shim.py
```

#### Success Criteria

- [ ] KoboldCPP responds on port 5001
- [ ] VITRIOL shim responds on port 5005
- [ ] Context rectification reduces 100k+ tokens to <7k
- [ ] Thermal polling returns valid temperature
- [ ] Full inference through shim completes without crash

---

### Phase 2: Brief Kernel Module Update

**Status:** Design Complete  
**Risk:** Low (stub mode, no hardware access)  
**Goal:** Correct BAR mapping for hardware discovery

#### Key Changes

| File | Change | Reason |
|------|--------|--------|
| `vitriol_new_ffi.bv` | `BAR_0` → `BAR_1` | VRAM is on BAR 1 (256MB) |
| `kernel.toml` | Add `pci_resource_len`, `pci_resource_start` | Aperture validation |
| `sentinel.bv` | New file | Hardware monitoring sentinels |
| `window.bv` | New file | Sliding window implementation |

#### Updated FFI Bindings (`kernel.toml`)

```toml
# Thermal monitoring
[thermal_zone_get_temp]
sig = "thermal_zone_get_temp(zone: UInt) -> Int"

# PCIe resource validation
[pci_resource_len]
sig = "pci_resource_len(dev: UInt, bar: UInt) -> UInt"

[pci_resource_start]
sig = "pci_resource_start(dev: UInt, bar: UInt) -> UInt"

# DMA/IOMMU helpers
[dma_map_single]
sig = "dma_map_single(dev: UInt, addr: UInt, size: UInt, dir: UInt) -> UInt"

[dma_mapping_error]
sig = "dma_mapping_error(dma_addr: UInt) -> Bool"

# Memory barriers
[mb]
sig = "mb() -> Unit"
[rmb]
sig = "rmb() -> Unit"
[wmb]
sig = "wmb() -> Unit"
```

#### Sentinel Transactions (`sentinel.bv`)

```brief
// Thermal Watcher
rct txn thermal_watch [true][gpu_temp < MAX_TEMP && !safety_halt] {
    let temp = thermal_zone_get_temp(0);
    &gpu_temp = temp;
    [temp >= MAX_TEMP] { &safety_halt = true; };
    term;
};

// Aperture Validator
rct txn validate_aperture [gpu_dev > 0 && bar_size == 0][bar_size > 0] {
    let size = pci_resource_len(gpu_dev, 1);
    &bar_size = size;
    [size <= 256MB] { &sliding_window_required = true; };
    term;
};

// IOMMU Canary
rct txn validate_iommu [dma_mapped == false][dma_mapped == true || safety_halt] {
    let test_addr = dma_alloc_coherent(gpu_dev, 4096, 0, 1);
    let dma_handle = dma_map_single(gpu_dev, test_addr, 4096, 3);
    let is_error = dma_mapping_error(dma_handle);
    [is_error == true] { &safety_halt = true; };
    term;
};
```

#### Testing Procedure

```bash
# Rebuild kernel module
cd ~/Desktop/Projects/linux-pipe-module
make clean && make

# Load in stub mode (SAFE)
sudo insmod vitriol_new_ffi.ko test_mode=1

# Check dmesg for sentinel output
dmesg | grep VITRIOL

# Unload
sudo rmmod vitriol_new_ffi
```

#### Success Criteria

- [ ] Module compiles without errors
- [ ] Stub mode loads successfully
- [ ] Sentinel transactions fire correctly
- [ ] Contracts enforce safety halts
- [ ] Brief compiler validates all pre/post conditions

---

### Phase 3: Hardware Discovery

**Status:** Pending Phase 2  
**Risk:** Medium (reads hardware, no writes)  
**Goal:** Verify BAR mapping without DMA

#### Configuration Changes

1. **GRUB Update (COMPLETED)**
   ```bash
   GRUB_CMDLINE_LINUX_DEFAULT="... intel_iommu=on iommu=pt"
   ```

2. **Module Parameter**
   ```bash
   sudo insmod vitriol_new_ffi.ko test_mode=0
   ```

#### Monitoring Commands

```bash
# Watch kernel logs in real-time
dmesg -w

# Check GPU status
nvidia-smi dmon -i 0

# Verify IOMMU groups
find /sys/kernel/iommu_groups/ -type l
```

#### Expected Output

```
[  123.456789] VITRIOL: Visita Interiora Terrae...
[  123.456790] VITRIOL: GPU device found (10de:1b82)
[  123.456791] VITRIOL: BAR 1 (VRAM) mapped at 256MB aperture
[  123.456792] VITRIOL: Legacy 256MB aperture - sliding window enabled
[  123.456793] VITRIOL: IOMMU passthrough active
```

#### Failure Modes

| Symptom | Likely Cause | Recovery |
|---------|-------------|----------|
| No VITRIOL output | Module didn't load | Check `dmesg | tail` |
| "BAR mapping failed" | Wrong BAR index | Verify `lspci -v` |
| "IOMMU error" | VT-d not enabled | Check GRUB, reboot |
| Screen freeze | BAR 0 mapped (framebuffer) | Hard reboot, use BAR 1 |

#### Success Criteria

- [ ] GPU device detected by vendor:device ID
- [ ] BAR 1 maps successfully (256MB)
- [ ] Sliding window flag set correctly
- [ ] IOMMU validation passes
- [ ] No system instability during mapping

---

### Phase 4: DMA Streaming

**Status:** Pending Phase 3  
**Risk:** High (actual DMA writes to VRAM)  
**Goal:** Stream layer from NVMe to GPU

#### Sliding Window Implementation (`window.bv`)

```brief
rct async txn stream_layer_sliding [
    !dma_active && 
    !safety_halt && 
    gpu_temp < MAX_TEMP
][dma_active == false] {
    
    let chunks = ceil(layer_size / APERTURE_SIZE);
    
    for (chunk = 0; chunk < chunks; chunk++) {
        let offset = chunk * APERTURE_SIZE;
        let size = min(layer_size - offset, APERTURE_SIZE);
        
        // Memory barrier before DMA
        wmb();
        
        // DMA transfer for this chunk
        pci_dma_copy(
            ssd_phys_addr + offset,
            gpu_bar_addr + offset,
            size
        );
        
        // Barrier after DMA
        rmb();
    };
    
    term;
};
```

#### Memory Barrier Strategy

| Barrier | Placement | Purpose |
|---------|-----------|---------|
| `wmb()` | Before DMA start | Ensure all writes visible to GPU |
| `mb()` | After DMA complete | Prevent CPU reordering |
| `rmb()` | Before reading DMA result | Ensure DMA data visible to CPU |

#### Test Sequence

1. **Small Buffer Test (4KB)**
   ```bash
   # Allocate coherent buffer
   # DMA from SSD to GPU
   # Verify data matches
   ```

2. **Single Chunk Test (256MB)**
   ```bash
   # Stream one full aperture
   # Verify no PCIe errors
   # Check GPU VRAM contents
   ```

3. **Multi-Chunk Test (512MB+)**
   ```bash
   # Stream layer requiring 2+ windows
   # Verify window remapping works
   # Check for data corruption at boundaries
   ```

4. **Full Layer Test (400MB for 9B model)**
   ```bash
   # Stream complete layer
   # Trigger inference
   # Verify model output is correct
   ```

#### Safety Protocols

1. **Before Loading Module:**
   ```bash
   # Flush all pending writes
   sync
   
   # Note current time for dmesg filtering
   date
   ```

2. **During Testing:**
   - Keep `dmesg -w` running in separate terminal
   - Monitor `nvidia-smi dmon` for power spikes
   - Have physical reset button accessible

3. **Emergency Recovery:**
   ```bash
   # If screen freezes but SSH works
   sudo rmmod vitriol_new_ffi
   
   # If completely unresponsive
   # Hold power button 5 seconds
   
   # After reboot, check for corruption
   fsck /dev/sdX
   ```

#### Success Criteria

- [ ] 4KB DMA transfer completes without error
- [ ] 256MB chunk transfers at PCIe bandwidth
- [ ] Sliding window remaps correctly
- [ ] No data corruption at chunk boundaries
- [ ] Full layer streaming enables 32B+ model inference

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                    OpenCode / Agent Clients                      │
│              (sending 389k+ character contexts)                  │
└─────────────────────────┬────────────────────────────────────────┘
                          │ HTTP POST /v1/chat/completions
                          ▼
┌──────────────────────────────────────────────────────────────────┐
│              VITRIOL Shim (port 5005, Python)                    │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  1. Thermal Poll: nvidia-smi → halt if >85°C               │  │
│  │  2. Calcination: Drop middle messages, keep last 4         │  │
│  │  3. Sublimation: Strip <reasoning>, tool_results           │  │
│  │  4. Coagulation: Enforce 7k token cap                      │  │
│  └────────────────────────────────────────────────────────────┘  │
│              Output: 7k tokens (rectified, safe)                 │
└─────────────────────────┬────────────────────────────────────────┘
                          │ Forward rectified request
                          ▼
┌──────────────────────────────────────────────────────────────────┐
│              KoboldCPP (port 5001, CUDA)                         │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  Qwen3.5 9B Q4_K_M (5.5GB)                                 │  │
│  │  GPU Layers: 30 (GTX 1070 Ti)                              │  │
│  │  Context: 8192 tokens                                      │  │
│  │  Quantized KV: 1 (reduces VRAM)                            │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
                          │
                          ▼ (Phase 2+)
┌──────────────────────────────────────────────────────────────────┐
│         VITRIOL Kernel Module (Brief-compiled C)                 │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  SENTINELS:                                                │  │
│  │  - Thermal Watcher (polls every transaction)               │  │
│  │  - Aperture Validator (runs once at init)                  │  │
│  │  - IOMMU Canary (validates DMA mapping)                    │  │
│  │                                                            │  │
│  │  DMA ENGINE:                                               │  │
│  │  - Sliding Window (256MB chunks for BAR 1)                 │  │
│  │  - Memory Barriers (wmb/mb/rmb)                            │  │
│  │  - Physical Address Translation (dma_map_single)           │  │
│  └────────────────────────────────────────────────────────────┘  │
└─────────────────────────┬────────────────────────────────────────┘
                          │ PCIe P2P DMA
                          ▼
┌────────────────┐     ┌────────────────┐
│     NVMe SSD   │────▶│   GPU VRAM     │
│  (Model Weights)     │   (8GB GTX)    │
└────────────────┘     └────────────────┘
```

---

## File Structure

```
linux-pipe-module/
├── VITRIOL_IMPLEMENTATION_PLAN.md    # This document
├── KOBOLDCPP_INTEGRATION.md          # Shim integration guide
├── README.md                         # Project overview
│
├── # Phase 1: Context Rectification
├── vitriol_shim.py                   # Python proxy (port 5005)
├── test_shim.py                      # Shim test suite
├── safe_test_vitriol.sh              # Graduated testing
│
├── # Phase 2: Brief Kernel Module
├── vitriol_new_ffi.bv                # Main Brief source
├── sentinel.bv                       # Hardware sentinels (TODO)
├── window.bv                         # Sliding window (TODO)
├── kernel.toml                       # FFI bindings
├── Makefile                          # Kernel build
│
├── # Phase 3: Userspace Daemon
├── vitriol-daemon/
│   ├── src/main.rs                   # Rust socket server
│   └── Cargo.toml
│
└── # Python Client Library
    └── libvitriol/
        ├── __init__.py
        ├── client.py
        └── types.py
```

---

## Testing Checklist

### Pre-Test Preparation

- [ ] Backup important data (in case of kernel panic)
- [ ] Close all unnecessary applications
- [ ] Open `dmesg -w` in separate terminal
- [ ] Note physical reset button location
- [ ] Have internet accessible (for troubleshooting)

### Phase 1 Tests (Python Shim)

- [ ] `./safe_test_vitriol.sh` - Stage 1 passes (KoboldCPP)
- [ ] `./safe_test_vitriol.sh` - Stage 2 passes (VITRIOL health)
- [ ] `./safe_test_vitriol.sh` - Stage 3 passes (full inference)
- [ ] `python3 test_shim.py rectify` - Context reduction works
- [ ] OpenCode connects to port 5005 successfully
- [ ] No OOM crashes with bloated prompts

### Phase 2 Tests (Brief Module - Stub Mode)

- [ ] `make` compiles without errors
- [ ] `sudo insmod vitriol_new_ffi.ko test_mode=1` loads
- [ ] `dmesg | grep VITRIOL` shows expected output
- [ ] `sudo rmmod vitriol_new_ffi` unloads cleanly
- [ ] No kernel warnings or errors

### Phase 3 Tests (Hardware Discovery)

- [ ] `sudo insmod vitriol_new_ffi.ko test_mode=0` loads
- [ ] GPU device found: `10de:1b82`
- [ ] BAR 1 mapped at 256MB
- [ ] Sliding window flag set
- [ ] IOMMU validation passes
- [ ] System remains stable for 5+ minutes

### Phase 4 Tests (DMA Streaming)

- [ ] 4KB DMA transfer successful
- [ ] 256MB chunk transfer successful
- [ ] Sliding window remaps correctly
- [ ] Multi-chunk layer (400MB) streams without corruption
- [ ] Inference with streamed layer produces correct output
- [ ] No system instability after 10+ DMA operations

---

## Troubleshooting Guide

### Common Issues

#### Issue: "Module load failed: Exec format error"
**Cause:** Kernel version mismatch  
**Solution:** Rebuild module after kernel update
```bash
make clean && make
```

#### Issue: "No VITRIOL output in dmesg"
**Cause:** Module loaded but printk suppressed  
**Solution:** Increase log level
```bash
sudo dmesg -n 8
sudo insmod vitriol_new_ffi.ko test_mode=1
```

#### Issue: "PCIe Bus Error" in dmesg
**Cause:** Attempted to map >256MB aperture  
**Solution:** Verify `sliding_window_required == true` and use chunked streaming

#### Issue: Screen freezes after module load
**Cause:** Mapped BAR 0 (framebuffer) instead of BAR 1  
**Solution:** Hard reboot, change `BAR_VRAM` to `1` in Brief source

#### Issue: "IOMMU mapping error"
**Cause:** VT-d not enabled or IOMMU not in passthrough mode  
**Solution:** Verify GRUB has `intel_iommu=on iommu=pt`, reboot

#### Issue: Shim returns "GPU thermal limit exceeded"
**Cause:** `nvidia-smi` polling failed or temp >= 85°C  
**Solution:** Check GPU cooling, verify `nvidia-smi` works manually

---

## Performance Targets

### Context Rectification (Phase 1)

| Metric | Target | Measurement |
|--------|--------|-------------|
| Token Reduction | 90%+ | 100k → <10k tokens |
| Latency Added | <50ms | Shim processing time |
| Thermal Polling | <100ms | `nvidia-smi` call overhead |
| Crash Prevention | 100% | No OOM with bloated prompts |

### DMA Streaming (Phase 4)

| Metric | Target | Measurement |
|--------|--------|-------------|
| PCIe Throughput | 12 GB/s | `dd` benchmark over DMA |
| Window Remap Time | <1ms | `pci_iounmap` + `pci_iomap` |
| Layer Stream (400MB) | <100ms | Total transfer time |
| Inference Speed | 40+ tok/s | Qwen 9B with streaming |

---

## Safety Notes

### Kernel Development Risks

1. **Kernel Panic:** Incorrect kernel code can crash the entire system
   - Mitigation: Always test in `test_mode=1` first
   - Mitigation: Keep `sync` command handy to flush filesystem

2. **Hardware Conflicts:** Mapping wrong BAR can corrupt display/VRAM
   - Mitigation: Use BAR 1 (VRAM), never BAR 0 (framebuffer)
   - Mitigation: Run on iGPU if possible (display isolation)

3. **Data Corruption:** DMA to wrong address can corrupt files
   - Mitigation: IOMMU passthrough provides hardware fence
   - Mitigation: Backup before Phase 3+ testing

### Operational Safety

```bash
# Before any kernel testing
sync  # Flush filesystem buffers

# Monitor for hardware errors
dmesg -w | grep -e "error" -e "VITRIOL"

# Check GPU health
watch -n 1 nvidia-smi

# Emergency unload
sudo rmmod vitriol_new_ffi
```

---

## Next Steps

### Immediate (Today)

1. **Test Phase 1:** Run `./safe_test_vitriol.sh`
2. **Verify Shim:** Confirm context rectification works
3. **Point OpenCode:** Configure to use port 5005

### Short-Term (This Week)

1. **Implement Sentinels:** Write `sentinel.bv` with thermal/aperture/IOMMU checks
2. **Add Sliding Window:** Implement `window.bv` for 256MB chunking
3. **Test Hardware Discovery:** Load module with `test_mode=0`

### Medium-Term (This Month)

1. **Enable DMA:** Phase 4 testing with small buffers
2. **Integrate with Daemon:** Connect Rust daemon to kernel module
3. **Full Stack Test:** End-to-end inference with layer streaming

### Long-Term (Future)

1. **Optimize Performance:** Reduce window remap overhead
2. **Support Larger Models:** Enable 32B/72B inference
3. **Multi-GPU Support:** Extend to multiple GPUs for parallel streaming

---

## References

- **Brief Language:** https://github.com/Randozart/brief-lang
- **KoboldCPP:** https://github.com/LostRuins/koboldcpp
- **Linux Kernel DMA:** https://docs.kernel.org/dma-api.html
- **PCIe BAR Mapping:** https://wiki.osdev.org/PCI
- **IOMMU Guide:** https://www.kernel.org/doc/html/latest/IOMMU.html

---

## Revision History

| Version | Date | Changes |
|---------|------|---------|
| 1.0 | 2026-04-30 | Initial implementation plan |
| 2.0 | 2026-05-09 | Added hardware analysis, BAR findings, IOMMU config, four guardrails |

---

**Status:** Phase 1 Ready - Begin testing `vitriol_shim.py` with KoboldCPP
