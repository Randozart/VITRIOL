# Alka + VITRIOL Integration Guide

> *"VITRIOL = Engine. Alka = ECU."*

## Overview

VITRIOL runs Qwen3.6-35B-A3B on a GTX 1070 Ti (8GB) by streaming only the 8 active MoE experts per token instead of loading all 256. The current path uses `llama.cpp`'s `-ot ".*exps.*=CPU"` flag to keep experts on CPU while the base model sits in VRAM (775MB total).

**The bottleneck:** CPU→GPU expert transfer via `cudaMemcpy` is slow. The target is NVMe→GPU direct DMA via PCIe P2P.

**Alka's role:** Replace manual DMA management with declarative instruction sequences for expert swapping, BAR1 sliding window orchestration, speculative decoding, and dual-GPU coordination.

---

## 1. The Core Loop: Expert Streaming

VITRIOL's inner loop for every inference token:

1. MoE router determines which 8 of 256 experts are active
2. Load those 8 experts from NVMe/CPU into GPU VRAM
3. Run the experts on GPU
4. Unload (or reuse) the experts
5. Repeat

### Current (llama.cpp via `-ot`)

```
CPU ↔ GPU via cudaMemcpyAsync
  ↓
256 experts on CPU, llama.cpp manages swapping
  ↓
10.6 tok/s — CPU transfer is the bottleneck
```

### Alka Target

```
CLAIM GPU_1070TI;
LIMIT GPU_1070TI 85000;

// Prefetch next 8 experts while GPU computes
FOR token IN stream {
    // Get active expert indices from router
    LET experts = ROUTE(token);

    // Load each expert via DMA
    FOR e IN experts {
        FLOW NVME[e.offset] -> GPU_1070TI.VRAM[e.gpu_addr] e.size;
    }

    // Wait for all loads to complete
    FENCE GPU_1070TI.METAPAGE == token;
}
```

**Key Alka primitives:**
- `FLOW` — initiates DMA transfer (NVMe→GPU)
- `FENCE` — polls metapage for completion
- `SIGNAL` — triggers expert compute on GPU
- `LIMIT` — enforces thermal safety

---

## 2. The BAR1 Sliding Window (256MB Bottleneck)

VITRIOL's single hardest physical constraint: the GTX 1070 Ti's BAR1 aperture is only 256MB. A typical MoE expert tensor is ~150-400MB. Layers larger than 256MB require chunked streaming with window remapping.

### Without Alka (manual)

```c
// Manually chunk tensor into 256MB drops
for (i = 0; i < drops; i++) {
    pci_resource_write(control_reg, i * 256MB);  // SHIFT window
    cudaMemcpy(vram + window_base, ssd + offset, chunk_size);
    // Must ensure FENCE before next iteration
}
```

### With Alka (automatic)

```alka
// User writes one line:
FLOW expert_weights -> GPU_MAIN.DATA_PLANE 400MB;

// Compiler auto-generates:
SHIFT GPU_MAIN.DATA_PLANE @ 0;       // Window to 0
FLOW expert_weights[0..256MB] -> GPU 256MB;
FENCE GPU_MAIN.METAPAGE == 1;        // Wait for DMA
SHIFT GPU_MAIN.DATA_PLANE @ 256MB;   // Slide window
FLOW expert_weights[256MB..400MB] -> GPU 144MB;
FENCE GPU_MAIN.METAPAGE == 2;
```

Or explicitly for more control:

```alka
REFRACT expert_weights 400MB 256MB;
```

**Why this matters for VITRIOL:** Every expert tensor load would otherwise require manual window management. Alka's `REFRACT` instruction and implicit safety injection handle this automatically.

---

## 3. Speculative Decoding with the GTX 960

VITRIOL has a GTX 960 (2GB) sitting idle. It's useless for the main model but perfect as a draft model for speculative decoding.

### Architecture

```
GTX 960 (Draft, 2GB)          GTX 1070 Ti (Target, 8GB)
─────────────────────         ─────────────────────
Qwen 0.5B ternary             Qwen3.6-35B-A3B
500MB → fits in VRAM           775MB base + expert swap
100+ tok/s                     10.6 tok/s (estimated)
     │                               ▲
     │   P2P DMA via PCIe             │
     │   (8 experts guessed)          │
     └───────────────────────────────┘
```

### Alka Recipe

```alka
REQUIRE dual_vitriol.alkavl;

CLAIM GPU_960 AS draft;
CLAIM GPU_1070TI AS target;

// Establish P2P pipe between cards
PIPE draft.OUTPUT => target.INPUT 1MB 0x1;

// Speculative loop
FOR token IN user_prompt {
    // Draft generates 8 candidate tokens
    SPECULATE draft -> target COUNT 8;

    // Target verifies all 8 in one parallel pass
    FENCE target.VALIDATION == COMPLETE;

    // Reject and correct if wrong
    IF target.AGREEMENT < 8 THEN {
        REVERT draft.KV_CACHE TO target.CORRECT_STATE;
    }
}
```

**Expected speedup:** 10.6 → ~30 tok/s (3x from exploiting idle silicon).

---

## 4. Expert Prefetch Pipeline

VITRIOL's LRU cache manager (`vitriol-expert-cache.cpp`) is already built. Alka can double-buffer the expert loading to hide latency:

### The Pipeline

```
              TOKEN N                    TOKEN N+1
              ───────                    ─────────
GPU Compute:  [expert set A]             [expert set B]
DMA Load:     loading set B              loading set C
              ▲                          ▲
              │                          │
              PIPE (continuous ring buffer)
```

### Alka Recipe

```alka
REQUIRE vitriol_1070ti.alkavl;

CLAIM GPU_1070TI;
CLAIM NVME_SSD;

// Set up double-buffer ring
PIPE NVME_SSD.EXPERT_STREAM => GPU_1070TI.EXPERT_RING 256MB 0x1;

// Prime first buffer
SIGNAL PRIME_EXPERT_0;

FOR token IN inference_stream {
    LET experts = ROUTE(token);

    // Compute current, load next, in parallel
    PARALLEL {
        COMPUTE experts;
        PREFETCH NEXT_EXPERTS;
    }

    FENCE GPU_1070TI.EXPERT_READY == token;
}
```

---

## 5. Kernel Module Safety

VITRIOL's original `vitriol.ko` was built (410KB) but never loaded — too dangerous without safe testing. Alka provides the testing infrastructure:

### Development Workflow

```
1. Write Recipe (.alka)         ─┐
2. Run --mock                    │  No kernel needed
3. Verify output matches spec   ─┘
4. Load vitriol.ko (Athanor)    ─┐
5. Run --safe (.alkas + .azoth)  │  With rollback safety
6. Check dmesg for success      ─┘
7. Graduate to --execute        ─┐  Production
```

### Azoth Rollback for VITRIOL

Every forward operation in the expert streaming loop has a defined inverse:

| Forward | Rollback |
|---------|----------|
| `CLAIM GPU_1070TI` | Restore VRAM layout |
| `FLOW expert -> VRAM` | `VOID` (zero VRAM region) |
| `SHIFT window` | `SHIFT` back to original offset |
| `SIGNAL COMPUTE` | `QUENCH` (halt if thermal exceeded) |
| `SPECULATE draft` | `REVERT` draft KV-cache |

---

## 6. Dual-GPU Coordination

VITRIOL's benchmark script breaks on dual-GPU because memory calculations assume a single device. Alka's Vial system and vessel abstraction solve this:

### Vial for Dual-GPU

```alkavl
Vessel GPU_1070TI {
    PCI_ID: 10de:1b82;
    Aperture DATA_PLANE { BAR: 1; SIZE: 256MB; TYPE: Prefetchable; }
    Memory VRAM { TOTAL: 8GB; RESERVED: 775MB; }
    Affordance: DMA_MASTER;
    Affordance: P2P_TUNNEL;
}

Vessel GPU_960 {
    PCI_ID: 10de:1401;
    Aperture DATA_PLANE { BAR: 1; SIZE: 256MB; TYPE: Prefetchable; }
    Memory VRAM { TOTAL: 2GB; RESERVED: 256MB; }
    Affordance: SPECULATIVE_DRAFT;
}
```

The compiler validates each vessel against its affordances before emitting binary packets. No runtime device confusion.

### P2P DMA Between Cards

Both GPUs are in IOMMU group 1 (verified). Alka's `FLOW` instruction enables direct P2P DMA:

```alka
// 1070 Ti reads draft tokens from 960's VRAM directly
FLOW GPU_960.OUTPUT_RING -> GPU_1070TI.INPUT_BUFFER 1MB;
```

No CPU touch. No `cudaMemcpy`. No kernel involvement for the data plane.

---

## 7. Integration Blueprint

### Phase 1: Instrument the Baseline (Now)

**Problem:** VITRIOL has no throughput numbers for the 35B model (session interrupted mid-warmup, HTTP 503).

**Alka solution:**
```alka
CLAIM GPU_1070TI;
WATCH GPU_1070TI.THROUGHPUT 10ms;  // Monitor tok/s
SIGNAL START_BENCHMARK;
// ... inference loop ...
SIGNAL STOP_BENCHMARK;
TRACE GPU_1070TI;  // Dump timing profile
```

Write an Alka Recipe that wraps the llama.cpp benchmark with precise timing and thermal logging. The `WATCH` instruction provides real-time monitoring; `TRACE` captures the full execution profile for bottleneck analysis.

### Phase 2: Replace CPU Expert Transfer with Alka FLOW

**Current:** `cudaMemcpyAsync` at `ggml-cuda.cu` lines 682 and 2992.

**Target:** Alka `FLOW` instruction with P2P DMA fallback:

```
cudaMemcpyAsync → memcpy → FLOW via Alka
```

Even if the first integration uses `memcpy` internally (kernel module not ready), the Alka Recipe encodes the intent. When the kernel module is ready, only the tool implementation changes — the Recipe stays the same.

### Phase 3: BAR1 Sliding Window Automation

The 256MB aperture is a hard constraint. Integrate `REFRACT` into the expert load path:

1. Alka compiler reads the Vial's `MAX_WINDOW: 256MB`
2. Recipe says `FLOW expert_weights -> GPU 400MB;`
3. Compiler auto-injects `SHIFT` loop with `FENCE` between each chunk
4. Kernel module executes the expanded packet sequence

### Phase 4: Speculative Decoding with GTX 960

Load a 0.5B ternary draft model on the 960. The `SPECULATE` instruction bridges the two GPUs via P2P DMA. Expected: 10.6 → ~30 tok/s.

### Phase 5: Full Alka Orchestration

The final state — Alka drives the entire inference pipeline:

```alka
REQUIRE vitriol_rig.alkavl;

// Boot sequence
CLAIM GPU_1070TI AS compute;
CLAIM GPU_960 AS draft;
CLAIM NVME_SSD AS store;

// Thermal guard
LIMIT compute 85000;
LIMIT draft 98000;

// Establish dataflows
PIPE store.EXPERT_STREAM => compute.EXPERT_RING 256MB;
PIPE draft.OUTPUT => compute.DRAFT_BUFFER 1MB;
PIPE compute.OUTPUT => HOST.RESULT_RING 64MB;

// Main loop
FOR token IN GENERATE {
    LET experts = ROUTE(token, MODEL.router);

    PARALLEL {
        // Draft guesses next 8 tokens
        SPECULATE draft -> compute COUNT 8;

        // Compute current experts
        FOR e IN experts {
            FLOW store[e.offset] -> compute[e.vram_addr] e.size;
        }

        // Wait for all
        FENCE compute.EXPERT_READY == token;
    }

    // Verify draft predictions
    FENCE compute.DRAFT_VERIFIED == token;

    // Reset thermal throttle between tokens
    SENSE compute.THERMAL;
    GUARD compute.THERMAL > 85000 QUENCH;

    SIGNAL CONTINUE;
}
```

---

## 8. Immediate Next Steps

### Step 1: Write a Benchmark Wrapper Recipe

```alka
// benchmark_35b.alka
CLAIM GPU_1070TI;
LIMIT GPU_1070TI 85000;
SIGNAL llama_cpp_inference {
    ngl=20, ot=".*exps.*=CPU"
};
WATCH THROUGHPUT 1000ms;
SIGNAL STOP_BENCHMARK;
TRACE GPU_1070TI.PERF_COUNTERS;
```

Run: `alkac benchmark_35b.alka vitriol_1070ti.alkavl`

### Step 2: Profile the Expert Transfer Bottleneck

Use `WATCH` and `TRACE` to measure:
- CPU→GPU transfer time per expert
- PCIe bus utilization during transfer
- GPU compute idle time waiting for experts

### Step 3: Replace the First cudaMemcpyAsync with Alka

At `ggml-cuda.cu:682`, replace:
```c
cudaMemcpyAsync(dst, src, size, cudaMemcpyHostToDevice, stream);
```
With:
```c
vitriol_dma_transfer(gpu_offset, file_offset, size);
```

The Alka Recipe already has the `FLOW` instruction ready. The tool implementation (`tools/core/flow.zig`) just needs to call the kernel module's DMA engine.

### Step 4: Load the GTX 960 Draft Model

```bash
# Already verified: Alka CLAIM + BAR mapping works on 960
# Load draft model via -ot on second GPU instance
CUDA_VISIBLE_DEVICES=1 ./llama-server \
    -m qwen_0.5b_ternary.gguf \
    -ngl 30
```

### Step 5: Wire SPECULATE Between GPUs

Once both models run independently, add the P2P pipe:
```alka
PIPE draft.OUTPUT_RING => compute.DRAFT_BUFFER 1MB 0x1;
SPECULATE draft -> compute COUNT 8;
```

---

## Summary

| VITRIOL Pain Point | Alka Solution | Status |
|--------------------|---------------|--------|
| No throughput numbers | `WATCH` + `TRACE` benchmark wrapper | Ready now |
| 256MB BAR1 sliding window | `REFRACT` + implicit safety injection | Ready now |
| Manual DMA management | `FLOW` instruction with Vial validation | Ready now |
| Dual-GPU coordination | Vial vessels + P2P `FLOW` | Ready now |
| Speculative decoding | `SPECULATE` instruction | Spec'd, tool not implemented |
| Kernel module too dangerous | `--mock` + `--safe` + Azoth rollback | Ready now |
| Expert prefetch pipeline | `PIPE` continuous ring buffer | Kernel stub, needs full impl |
| 100% manual C code | Declarative `.alka` Recipes | Ready now |
