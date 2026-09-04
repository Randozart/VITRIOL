# VITRIOL Intel Integration Plan

*"There are no impossibilities. Someone somewhere in the world has found an optimization, and we shall use it."*

## The Hardware

| Component | Capability | Implication |
|-----------|-----------|-------------|
| Intel Core Ultra X7 358H | 16c/16t, AVX2+AVX-VNNI | Quantized INT8 GEMM on CPU |
| Intel Arc B390 (Xe3 iGPU) | 12 Xe3 cores, 122 TOPS INT8, XMX matrix engines | Dedicated matrix hardware — currently unused by llama.cpp |
| Intel NPU (Series 3) | 50 TOPS, ~2W idle | Always-on background inference |
| 64 GB LPDDR5X | ~85-140 GB/s shared | Entire model lives in RAM. CPU and GPU share physical memory. |

**The bottleneck is memory bandwidth, not compute.** At ~100 GB/s, a 13.5GB Q4 model takes 135ms just to read weights per token. Everything else is optimization around that wall.

## The Vision

VITRIOL becomes the inference operating system for Intel hardware — not a port, not a compatibility layer, but a native integration that makes every piece of silicon work. Three backends, each serving a purpose:

| Backend | Role | Hardware |
|---------|------|----------|
| **SYCL** | Primary GPU path | Arc B390 (XMX, USM zero-copy) |
| **Vulkan** | Fallback / comparison / no-oneAPI path | Arc B390 |
| **NPU** | Background / always-on / small models | Intel NPU |

The CPU is not a fallback. It's a co-processor with AVX-VNNI integer GEMM that can handle decode while the GPU does prefill.

## What Stock llama.cpp Leaves on the Table

1. **XMX matrix engines** — SYCL backend detects `ext_intel_matrix`, doesn't use it. 122 TOPS of INT8 hardware sitting dark.
2. **NPU** — Completely ignored. A 50 TOPS engine doing nothing.
3. **AVX-VNNI** — CPU path may not use VNNI integer dot product instructions.
4. **Shared memory** — All backends treat iGPU like discrete GPU. On Panther Lake, GPU reads the same physical RAM as CPU. Zero copies are possible.
5. **Graph capture** — `GGML_SYCL_GRAPH` exists but isn't tuned for Panther Lake.
6. **AOT compilation** — Could pre-compile for `intel_gpu_ptl_h`, eliminating JIT overhead.
7. **Bandwidth-aware scheduling** — Nobody routes ops based on which device has the best access pattern for that tensor.
8. **Heterogeneous execution** — GPU for prefill, CPU for decode, NPU for background. Research shows this beats any single device.

## The Plan: Seven Phases

### Phase 1: Vulkan Backend (Immediate — Zero Dependencies)

**Goal:** Get Qwen3.8-27B running on Arc B390 today.

**Why Vulkan first:** All dependencies already installed. Arc B390 detected by Vulkan. No oneAPI needed. Proves the shared-memory path works.

**Build:**
```bash
cd /home/prestopoverlord/Projects/VITRIOL/llama.cpp
cmake -B build-intel -DGGML_VULKAN=ON -DGGML_NATIVE=ON \
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF
cmake --build build-intel -j$(nproc)
```

**Build workaround (cmake 4.4+ FindVinux regression):**
cmake 4.4's `FindVulkan` module fails to auto-detect include/lib paths on some systems. Pass them explicitly:
```bash
cmake -B build-intel -DGGML_VULKAN=ON -DGGML_NATIVE=ON \
  -DVulkan_INCLUDE_DIR=/usr/include \
  -DVulkan_LIBRARY=/usr/lib/libvulkan.so \
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF
```
This is now wired into `scripts/build-llama-server.sh --backend vulkan`.

**Files to create/modify:**
- `scripts/build-llama-server.sh` — Add `--backend vulkan` flag
- `profiles/qwen38-intel-vulkan/config` — Arc B390 profile
- `profiles/qwen38-intel-vulkan/meta` — Profile metadata

**Expected:** ~3-5 tok/s for Qwen3.8-27B Q4_K_M. Not fast, but it works and validates the memory architecture.

### Phase 2: oneAPI Installation (User Action Required)

**Goal:** Unlock SYCL backend.

**Full install list (CachyOS/Arch):**
```bash
# Build tools (already installed)
sudo pacman -S cmake base-devel

# Vulkan backend dependencies (already installed on this system)
sudo pacman -S vulkan-headers vulkan-icd-loader vulkan-tools glslang spirv-tools spirv-headers shaderc

# SYCL backend dependencies (Phase 2)
yay -S intel-oneapi-dpcpp-cpp-compiler-runtime intel-oneapi-mkl-devel
# Or Intel official installer:
# wget https://registrationcenter-download.intel.com/akdlm/IRC_NAS/.../l_BaseKit_p_*.sh
# sudo sh l_BaseKit_p_*.sh --silent --eula accept
```

**Minimum required packages for SYCL:**
- `intel-oneapi-dpcpp-cpp-compiler-runtime` — DPC++ compiler (`icpx`)
- `intel-oneapi-mkl-devel` — oneMKL (required by SYCL CMakeLists, line 148)
- `intel-oneapi-tbb-devel` — TBB (may need newer than `onetbb` 2023.1.0)

**What user needs to sudo:**
```bash
# If using official installer:
sudo sh l_BaseKit_p_*.sh --silent --eula accept

# After install, source the environment:
source /opt/intel/oneapi/setvars.sh
```

### Phase 3: SYCL Backend (Production Intel Path)

**Goal:** Native Intel GPU path with XMX access, USM zero-copy, 60+ operations.

**Why SYCL over Vulkan:**
- Native Intel path (not a translation layer)
- Weight reorder optimization for Xe architecture (SOA layout)
- XMX matrix engine detection (opportunity to utilize)
- Full USM memory model (device/host/async)
- Explicit Panther Lake arch support (`intel_gpu_ptl_h/u`)
- 60+ operations vs Vulkan's similar count but with Intel-specific optimizations

**Build:**
```bash
source /opt/intel/oneapi/setvars.sh
cd /home/prestopoverlord/Projects/VITRIOL/llama.cpp
cmake -B build-sycl \
  -DGGML_SYCL=ON \
  -DGGML_NATIVE=ON \
  -DGGML_SYCL_TARGET=INTEL \
  -DGGML_SYCL_GRAPH=ON \
  -DGGML_SYCL_HOST_MEM_FALLBACK=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF
cmake --build build-sycl -j$(nproc)
```

**Files to create/modify:**
- `scripts/build-llama-server.sh` — Add `--backend sycl` flag
- `profiles/qwen38-intel-sycl/config` — SYCL-optimized profile
- `scripts/vitriol` — Intel device detection, `ONEAPI_DEVICE_SELECTOR`

### Phase 4: VITRIOL Integration (Make Intel a First-Class Citizen)

**Goal:** Hardware detection, profiles, launcher support for Intel.

**4a. Hardware Probe (`libvitriol/src/probe.rs`):**

Currently: nvidia-smi only.
Needed: Intel GPU + NPU detection.

```rust
// Intel GPU detection via DRM sysfs
fn intel_gpu_info() -> Vec<GpuInfo> {
    // Scan /sys/class/drm/card*/device/vendor for 0x8086
    // Read device ID, EU count from /sys/class/drm/card*/device/
    // Detect XMX via Vulkan or SYCL runtime query
}

// NPU detection
fn npu_info() -> Option<NpuInfo> {
    // Check /dev/accel/accel0 exists
    // Read NPU version from sysfs
}
```

Extend `GpuInfo` struct:
```rust
struct GpuInfo {
    vendor: GpuVendor,      // NVIDIA, INTEL, AMD
    name: String,
    driver: String,         // cuda, sycl, vulkan
    memory_bytes: u64,      // VRAM for discrete, allocated RAM for iGPU
    compute_tops: f64,      // TOPS for NPU, CUDA cores equivalent for GPU
    has_xmx: bool,         // Intel XMX matrix engines
    has_vnni: bool,         // CPU AVX-VNNI
    pcie_gen: Option<u32>,  // None for shared memory
    pcie_width: Option<u32>,
}
```

**4b. VRAM Estimator (`libvitriol/src/estimator.rs`):**

Currently: CUDA compute capability keyed.
Needed: Unified memory model for shared RAM.

For shared memory architectures:
```
"VRAM" = portion of system RAM allocated to model
Bandwidth = shared LPDDR5X (~100 GB/s), contention factor for CPU+GPU
Overhead = 0 (no transfer overhead, but bandwidth contention)
```

New formula for Intel iGPU:
```
Model_Weight_Budget = System_RAM * allocation_factor
  where allocation_factor = 0.7 (leave headroom for OS, KV cache, activations)

Bandwidth_Shared = LPDDR5X_bandwidth / contention_factor
  where contention_factor = 1.5 (CPU+GPU+OS sharing)

Theoretical_Max_Tok_s = Model_Weight_Size / Bandwidth_Shared
```

**4c. Launcher (`scripts/vitriol`):**

Extend for Intel:
- Detect Intel GPU: `cat /sys/class/drm/card*/device/vendor` (0x8086)
- Detect NPU: `ls /dev/accel/accel0`
- Device selection: `ONEAPI_DEVICE_SELECTOR` or `GGML_SYCL_VISIBLE_DEVICES`
- Config section `[intel]` for SYCL/NPU options
- Profile loading for Intel configs
- Remove CUDA assumptions in sparse-KV guard

**4d. Build System (`scripts/build-llama-server.sh`):**

```bash
# Usage:
./build-llama-server.sh --backend cuda    # NVIDIA (existing)
./build-llama-server.sh --backend sycl    # Intel SYCL
./build-llama-server.sh --backend vulkan  # Intel Vulkan (no oneAPI)
./build-llama-server.sh --backend auto    # Detect hardware, pick best
```

### Phase 5: XMX Matrix Engine (The Big Win)

**Goal:** Use Arc B390's XMX INT8 matrix engines for quantized GEMM.

**Current state:** SYCL backend detects `sycl::aspect::ext_intel_matrix` but comment says "currently, it's not used for XMX really."

**The opportunity:** XMX can do INT8 matrix multiply at 122 TOPS. For INT4 quantized models, we can pack weights into INT8 and use XMX.

**Implementation path:**
1. Query XMX capability via SYCL aspect
2. Implement XMX-accelerated GEMM kernel using `ext_intel_matrix` extension
3. INT4 weight × INT8 activation GEMM on XMX
4. This is the single biggest performance opportunity on this hardware

**Research needed:**
- Intel's oneAPI samples for XMX usage
- `sycl_ext_intel_matrix` extension documentation
- Existing XMX GEMM implementations in oneDNN or Compute Library

### Phase 6: NPU Integration (Always-On Background)

**Goal:** Small model on NPU for always-on agent tasks.

**The idea:** While the GPU sleeps or runs the main model, the NPU runs a small model (Qwen3-0.6B or Qwen3-1.7B) at ~2W for:
- Background summarization
- Code completion
- Always-on assistant wake word
- Prefetch/draft generation for speculative decoding

**Implementation:**
- OpenVINO backend with `GGML_OPENVINO_DEVICE=NPU`
- Profile: `qwen38-intel-npu` running Qwen3-0.6B
- VITRIOL launcher manages both GPU and NPU servers
- NPU model serves on a separate port, main model on primary port

**Speculative decoding with NPU:**
- NPU runs small draft model (~60 tok/s)
- GPU runs full Qwen3.8-27B target model
- NPU drafts, GPU verifies — net speedup over GPU alone

### Phase 7: Heterogeneous Scheduling (The Frontier)

**Goal:** Route operations to the best device for that workload.

**The research:**
- Prefill is compute-bound → GPU (XMX + EU parallelism)
- Decode is memory-bound → CPU may be better (AVX-VNNI, lower launch overhead)
- NPU for background tasks (2W idle)

**Implementation:**
- Stage-aware backend selection: GPU for prefill, CPU for decode
- Bandwidth-aware scheduling: route ops based on tensor access patterns
- Memory-bandwidth scheduling: weight-stationary ops → GPU, activation-heavy → CPU
- This requires modifying the ggml graph scheduler to support heterogeneous backends

**What this looks like in VITRIOL:**
```ini
[intel]
# Stage-aware scheduling
prefill_device = gpu       # Arc B390 for compute-bound prefill
decode_device = cpu        # AVX-VNNI for memory-bound decode
background_device = npu    # NPU for always-on tasks

# Memory allocation
model_ram_budget = 0.7     # 70% of 64GB = ~45GB for model + KV
kv_cache_device = gpu      # KV cache on GPU for fast attention
```

## Profile Templates

### qwen38-intel-vulkan
```ini
[gpu]
device = 0

[model]
path = ~/Downloads/Qwen3.8-27B-Q4_K_M.gguf
ngl = 99
context = 8192
threads = 8

[vitriol]
mode = off

[chimera]
mode = vulkan

[server]
host = 127.0.0.1
port = 8080
```

### qwen38-intel-sycl
```ini
[gpu]
device = 0

[model]
path = ~/Downloads/Qwen3.8-27B-Q4_K_M.gguf
ngl = 99
context = 8192
threads = 8

[vitriol]
mode = off

[server]
host = 127.0.0.1
port = 8080

[intel]
backend = sycl
aot_arch = intel_gpu_ptl_h
graph_capture = on
```

### qwen38-intel-npu
```ini
[model]
path = ~/Downloads/Qwen3-0.6B-Q4_0.gguf
ngl = 99
context = 4096
threads = 4

[vitriol]
mode = off

[server]
host = 127.0.0.1
port = 8081

[intel]
backend = openvino
device = NPU
```

## Research Log

Tracking optimization ideas from the community:

| Source | Optimization | Status | Impact |
|--------|-------------|--------|--------|
| Intel oneAPI docs | XMX matrix extension | Not yet implemented | +30-50% INT8 GEMM |
| llama.cpp SYCL backend | Weight reorder (SOA layout) | Already in code | Better memory access |
| llama.cpp SYCL backend | AOT compilation for PTL | Available via flag | Eliminates JIT overhead |
| llama.cpp SYCL backend | SYCL command graphs | Available via flag | Reduced launch overhead |
| llm.npu paper | NPU prefill offloading | Research | 22× faster prefill |
| NITRO paper | NPU inference via OpenVINO | Available | 10× faster than naive NPU |
| SpecPicks benchmarks | Arc B390 Vulkan: 3.5 tok/s Qwen3.8-27B | Measured | Baseline to beat |
| SpecPicks benchmarks | Arc B390 SYCL: ~18 tok/s Qwen3-8B | Measured | SYCL > Vulkan confirmed |
| OpenVINO 2026.3 | Graph fusion, kernel optimization | Available | General speedup |
| Intel Panther Lake NPU | 50 TOPS, Qwen3-8B ~20 tok/s | Measured | Background model viable |
| Panther Lake shared memory | Zero-copy via USM | Architecture fact | No transfer overhead |

## Success Criteria

| Phase | Metric | Target |
|-------|--------|--------|
| 1 | Qwen3.8-27B runs on Arc B390 | Any tok/s > 0 |
| 2 | oneAPI installed, SYCL builds | Clean compile |
| 3 | SYCL backend outperforms Vulkan | > 1.5× tok/s |
| 4 | `vitriol calibrate` detects Intel hardware | Correct detection |
| 5 | XMX GEMM kernel compiled and running | +20% over baseline SYCL |
| 6 | NPU runs Qwen3-0.6B in background | Separate server on NPU |
| 7 | GPU prefill + CPU decode heterogeneous | Measurable speedup |

## The Rule

There are no impossibilities. If someone, somewhere, has found a way to make a piece of hardware faster, we will find it and use it. The only limits are the ones we haven't researched yet.
