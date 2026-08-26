# VITRIOL Optimization Catalog — 2026-05-19

Comprehensive analysis of all proposed optimizations, prior art, and implementation
priority for Qwen3.6-35B-A3B on GTX 1070 Ti (8 GB VRAM, PCIe Gen3 x16).

---

## 1. LRU Cache — The Hypothetical Invasive Fix

### Current State
The llama.cpp LRU cache lives in `ggml_cuda_mul_mat_id`'s FP16 slow path (line 2597+
of `ggml-cuda.cu`). It is never reached for quantized models because the MMQ fast
path returns before the LRU code. See `docs/LRU_DIAGNOSTIC_FINDING.md` for full
diagnosis.

### Hypothetical Fix
To make the LRU cache work for quantized MoE models, we would need to:

**Option A: Force the slow path (not viable)**
- Remove the early return in `ggml_cuda_mul_mat_id` when VITRIOL stream mode is active.
- This forces all expert loads through the per-expert iteration loop where `vitriol_lru_ensure` lives.
- **Problem:** The MMQ fast path is 2-3× faster per expert than the slow path, even with
  LRU caching. MMQ fuses 8 experts into a single optimized kernel launch. The slow path
  launches one CUDA kernel per expert. The LRU saving on PCIe bandwidth would be
  dwarfed by the kernel launch overhead.

**Option B: Add LRU awareness to the MMQ path (invasive but viable)**
- Modify the MMQ kernel to accept a side-table of VRAM pointers alongside the VITRIOL
  page-locked host pointer.
- For each expert in the batch, check if it exists in the VRAM side-table. If yes,
  use the VRAM pointer instead of the host pointer.
- This requires: (a) modifying the MMQ CUDA kernel signature, (b) adding a VRAM cache
  lookup inside the kernel, (c) handling partial batches where some experts are cached
  and some aren't.
- **Problem:** This is effectively rewriting a core llama.cpp kernel. The same effect
  can be achieved at the VITRIOL level without touching llama.cpp internals.

**Why VITRIOL-level beating is better**
Rather than caching at the kernel level, Expert Pinning (section 2) achieves the same
effect at the orchestration layer — pre-identify frequent experts, load them into
VRAM once at startup, and skip their PCIe transfers at inference time. This is
architecture-agnostic, simpler to debug, and works with any kernel path.

---

## 2. Expert Pinning (PopFetcher / HOBBIT-style)

### Idea
MoE expert activation follows a Zipfian (power-law) distribution: a small fraction of
experts handles the majority of tokens. These "heavy hitters" (syntax experts for
`{`, `}`, `if`, `let`, `return`, etc.) fire on nearly every token. By pinning them in
VRAM permanently, we eliminate their PCIe transfer cost.

### Prior Art
- **PopFetcher** (cited in HOBBIT et al.): Proves MoE expert usage follows Zipfian
  distribution. A tiny fraction does 80% of work.
- **HOBBIT** (Tang et al., 2024, arXiv 2411.01433): Mixed-precision expert offloading
  on llama.cpp. Implements a "multidimensional expert caching policy" combining LRU,
  LFU, and priority-based eviction. Reports up to 9.93× decoding speedup on edge devices.
  Built on llama.cpp (~8000 lines C++), code not open-sourced.
- **Fate** (Fang et al., 2025, arXiv 2502.12224): "Shallow-favoring" expert caching
  strategy achieving 99% hit rate. Prioritizes shallow-layer experts where prediction
  accuracy is lower.

### Implementation Sketch

1. **Calibration pass:**
   - Run inference on 128K-256K tokens with expert activation logging
   - Record which expert IDs fire per layer, per token
   - Aggregate: count total activations per expert across all layers
   - Select top ~20 experts (at ~256 experts total, top 20 ≈ 8%)

2. **Pinning:**
   - Pre-allocate VRAM slots for pinned experts at startup (20 × ~40 MB ≈ 800 MB)
   - Load pinned experts into VRAM immediately after model load
   - Modify the VITRIOL expert fetch function:
     ```c
     void* get_expert_vram_ptr(int expert_id) {
         if (pinned_experts.contains(expert_id)) {
             return pinned_slots[pinned_experts[expert_id]].vram_ptr;
         }
         // Fall through to PCIe DMA from page-locked RAM
         return dma_from_host_buffer(expert_id);
     }
     ```
   - LRU eviction logic must skip pinned slots entirely

3. **Config:**
   ```ini
   [cache]
   pin_experts = 12,45,102,210,...
   pin_enabled = true
   ```

### Expected Gain
- **5-15%** gen speed improvement
- More stable token-to-token latency (no cold misses for frequent experts)
- VRAM cost: ~800 MiB for 20 pinned experts (well within the ~4877 MiB headroom)

### Complexity
Low. The VITRIOL cache slot structure already exists. Adding a `pinned` flag
and pre-load path is straightforward C++. No CUDA kernel changes needed.

---

## 3. Speculative Routing (Fate / PreScope-style Expert Prefetching)

### Idea
The router/gate for layer N+1 can be predicted from layer N's hidden state with >97%
accuracy (Fate). By running a cheap CPU-side predictor during GPU computation of
layer N's FFN, we can start DMA transfers for layer N+1's experts before the GPU
even knows it needs them. This overlaps PCIe latency with GPU math — hard latency
hiding.

### Prior Art
- **Fate** (Fang et al., 2025, arXiv 2502.12224):
  - Key insight: gate inputs from adjacent layers have cosine similarity >0.99.
  - Clone gating input to CPU, run next-layer gate in parallel with GPU.
  - 97.15% prefetch accuracy, 99% cache hit rate with shallow-favoring strategy.
  - Up to 4.1× decoding speedup.
  - **Tested on PCIe 3.0 x16** — identical bus to GTX 1070 Ti.
  - **Working third-party llama.cpp implementation**:
    `github.com/ongunm/llama-moe-cache`
    ~500 lines C++, 1.91× speedup reported on Qwen3-30B-A3B with 12GB GPU.

- **PreScope** (Yu et al., 2025, arXiv 2509.23638):
  - LLaPor: layer-aware lightweight predictor (0.5-2.8 MB per layer, 0.12-0.48 ms).
  - PreSched: cross-layer scheduler that optimizes prefetch vs on-demand tradeoff.
  - AsyncIO: pipelines transfers across heterogeneous memories, splits experts
    into fine-grained chunks to saturate PCIe.
  - 141% throughput improvement over Klotski, 74.6% latency reduction.
  - Tested on Qwen3-30B-A3B (same architecture family as Qwen3.6-35B-A3B).
  - No public code.

- **Pre-gated MoE** (Hwang et al., ISCA 2024, Microsoft):
  - Algorithm-system co-design. Pre-gating function alleviates dynamic expert
    activation, enabling pre-loading onto single GPU.
  - Improves performance while reducing GPU memory consumption.

- **MoE-Infinity** (Xue et al., 2024, arXiv 2401.14361):
  - Sparsity-aware expert cache with request-level activation tracing (EAMC).
  - 3.1-16.7× speedup on PCIe 4.0 systems.

### The Prediction Mechanics

Fate's key discovery: the gating network's input for layer N+1 is almost identical
to its input for layer N. This is because:

```
gate_input(N+1) = attention_output(N) = f(hidden_state(N))
gate_input(N)   = attention_output(N-1) = f(hidden_state(N-1))
```

And adjacent hidden states are highly correlated (cosine similarity >0.99) since
each layer makes a small refinement. So:

```
gate(hidden_state(N-1)) ≈ gate(hidden_state(N))
```

Meaning: while the GPU computes `attn(N) → gate(N) → ffn(N)`, the CPU can compute
`gate(predicted_hidden(N))` using the previous layer's output as a proxy.

### Implementation Sketch

1. **At the MoE layer boundary in `ggml_cuda_mul_mat_id`:**
   ```c
   // After GPU computes gate for layer N, but before FFN:
   
   // 1. Copy gating input to CPU-accessible buffer
   cudaMemcpyAsync(cpu_gate_input, gpu_gate_input, 
                   sizeof(float) * hidden_dim, 
                   cudaMemcpyDeviceToHost, vitriol_stream_d2h);
   
   // 2. On CPU: run predictor for layer N+1 while GPU does FFN
   // (This is where Fate's trick goes: gate weights are tiny, ~100KB)
   cudaStreamSynchronize(vitriol_stream_d2h);  // wait for copy
   int predicted_experts[8];
   vitriol_predict_experts(cpu_gate_input, layer_id + 1, predicted_experts);
   
   // 3. Submit DMA transfers for predicted experts on dedicated stream
   for (int i = 0; i < 8; i++) {
       int exp_id = predicted_experts[i];
       if (!vitriol_is_expert_in_vram(exp_id)) {
           cudaMemcpyAsync(vram_slot[exp_id], host_buffer[exp_id],
                           expert_size, cudaMemcpyHostToDevice, 
                           vitriol_stream_dma);  // background stream!
       }
   }
   ```

2. **Synchronization:**
   - At the start of layer N+1, sync the DMA stream
   - If experts are already in VRAM: zero wait
   - If not: fall through to on-demand DMA (prediction was wrong)

3. **Prediction accuracy management:**
   - Fate: 97% accuracy → 1 expert miss every 33 tokens → negligible overhead
   - Worst case: all 8 predictions wrong → same cost as no prefetching
   - CPU predictor cost: <1 ms per layer (gate weights are ~100KB, input is ~4096
     float16s, matmul is trivial)

### Expected Gain
- **50-90% overlap** of PCIe wait time with GPU computation
- Potential gen speed: **15-20 tok/s** (from current 10.96)
- The Fate llama.cpp fork shows 1.91× on similar hardware and model family

### Complexity
Medium. Requires:
- A dedicated CUDA stream for DMA prefetches (separate from compute stream)
- A CPU-side predictor function that mimics the MoE gate
- Careful synchronization to avoid races
- Graceful fallback when predictions miss

---

## 4. P-State Locking (The Anti-Stutter)

### Idea
NVIDIA GPUs drop from P0 (max performance) to P2/P8 (power save) during PCIe idle
periods. On Pascal, the wake-up from P8 to P0 takes ~10-50 ms. During MoE inference,
between expert DMA transfers, the GPU may briefly idle — just long enough to trigger
a P-state transition. The wake-up latency adds micro-stutter per token.

### Implementation
```python
import pynvml

def lock_p0():
    pynvml.nvmlInit()
    handle = pynvml.nvmlDeviceGetHandleByIndex(0)
    max_sm = pynvml.nvmlDeviceGetMaxClockInfo(handle, pynvml.NVML_CLOCK_SM)
    max_mem = pynvml.nvmlDeviceGetMaxClockInfo(handle, pynvml.NVML_CLOCK_MEM)
    pynvml.nvmlDeviceSetGpuLockedClocks(handle, max_sm, max_sm)
    pynvml.nvmlDeviceSetMemoryLockedClocks(handle, max_mem, max_mem)
```

### Risks
- Requires `sudo`/root (NVML clock lock is privileged)
- Sustained P0 increases heat and fan noise
- Possible thermal throttling if cooling is inadequate (blower card?)
- Minor long-term reliability concern (constant max voltage)

### Expected Gain
Unknown — needs benchmarking. The GTX 1070 Ti at 10.96 tok/s takes ~91 ms per token.
If ~60% (~55 ms) of that is PCIe wait time, the GPU may be idle enough to trigger
P-state transitions. If each transition costs 20 ms and happens every 5-10 tokens,
that's 2-4% lost throughput with high jitter.

### Complexity
Trivial. 10 lines of Python. Add as an optional `vitriol serve --lock-p0` flag.

---

## 5. io_uring Zero-Copy Bypass

### Idea
Use io_uring with fixed buffers to let the NVMe controller DMA directly into
VITRIOL's page-locked RAM, bypassing the kernel page cache entirely.

### Why Not Applicable
VITRIOL already loads the entire ~12 GB model into page-locked RAM at startup
(the "RAM shot"). During inference, all expert transfers are RAM → VRAM over PCIe.
There is no disk I/O during inference. io_uring would only help if we were loading
experts from NVMe on demand, which we are not.

### When it Would Matter
- If we switched to an SSD-only mode (no RAM shot), where experts are loaded from
  NVMe on demand rather than pre-loaded into RAM
- This would save ~10 GB of RAM but at the cost of NVMe → PCIe → VRAM latency
  (NVMe is ~3-7 GB/s sequential vs PCIe 15.76 GB/s — both slower than RAM)
- **Not recommended** — RAM shot is already the optimal strategy for 15 GB RAM

---

## 6. Bit-Plane Streaming (Progressive Weights)

### Idea
Split IQ2_M's 2.6 bpw quant into two 1-bit streams: stream the "sign" bit first for
an immediate rough computation, then stream the "magnitude" bit to refine. This
effectively doubles PCIe bandwidth at the cost of complex kernel logic.

### Why Not Feasible
- Requires custom CUDA kernels for progressive matmul (partial compute on 1-bit data,
  then refine with second bit)
- The GTX 1070 Ti (Pascal CC 6.1) lacks modern features like async copy,
  independent thread scheduling, and large shared memory
- IQ2_M is not a 2-bit format — it's a complex mixed-precision quant (some weights
  2-bit, some 4-bit, some 8-bit). Splitting it into bit-planes would require a
  completely new quant format and a full requantization pipeline
- Estimated engineering effort: months, with uncertain ROI
- **Defer indefinitely.** If the community develops a general progressive quant
  format for MoE, we can re-evaluate.

---

## 7. SP-MoE: MTP + Expert Prefetching Co-Design

### Idea
Combine MTP speculative decoding with expert prefetching. The draft model's attention
outputs can predict which experts the target model will activate during verification.
This gives extra I/O budget for prefetching because the draft model runs ahead of
the target model.

### Prior Art
- **SP-MoE** (Chen et al., 2025, arXiv 2510.10302):
  - First SD-aware expert offloading framework.
  - "Speculative expert prefetching": uses structural correspondence between draft
    and target models to predict experts.
  - "Cutoff-layer policy": bounds prefetch depth to avoid cache thrashing.
  - Pipelined runtime with async prefetch threads and batched I/O.
  - 1.07×-3.5× TPOT speedup over state-of-the-art methods.
  - No public code.

### Relevance to VITRIOL
VITRIOL already supports MTP (draft model) and has a standard expert prefetch path.
SP-MoE's insight — use the draft model to drive prefetching — is complementary to
Fate-style cross-layer prediction. The draft model is already computing attention
for future tokens, so its hidden states provide a "free" preview of what's coming.
This could be combined with Fate's predictor for even higher accuracy.

### Status
No public code. The paper is very recent (October 2025). Worth watching for a code
release. The architecture is directly applicable to VITRIOL.

---

## 8. Implementation Roadmap

### Priority Order

| # | Optimization | Est. Effort | Est. Gain | Risk | Depends On |
|---|-------------|-------------|-----------|------|------------|
| 1 | Expert Pinning | Weekend | 5-15% | Low | Calibration run |
| 2 | P-State Locking | 1 hour | 0-5% | Low | Benchmark verdict |
| 3 | Speculative Routing | 1-2 weeks | 50-90% overlap | Medium | Fate code study |
| 4 | SP-MoE (if code drops) | TBD | TBD | Medium | Public code release |
| — | LRU invasive fix | Deferred | — | — | Not competitive with pinning |
| — | io_uring | Not applicable | — | — | RAM shot already optimal |
| — | Bit-Plane | Not feasible | — | — | Pascal + quant format limits |

### Dependencies
- **Expert Pinning**: Independent. Needs calibration profiling logic (can be a
  separate script + config list).
- **P-State Locking**: Independent. Adds a one-shot setup script.
- **Speculative Routing**: Depends on understanding the Fate llama.cpp fork at
  `github.com/ongunm/llama-moe-cache`. Can be implemented independently of pinning
  but benefits from pinning (pinned experts are always in VRAM, never need fetching).
- **SP-MoE**: Depends on public code release. Building from paper alone would be
  research-level effort.

### Measurement Plan
Each optimization should be evaluated with:
- Gen speed (tok/s) at n=50 and n=100 (using `scripts/benchmark_mtp_sweep.sh` pattern)
- VRAM usage (MiB, via `nvidia-smi`)
- System RAM usage (GiB, via `free -h`)
- For Speculative Routing: also measure prediction accuracy and prefetch hit rate
- A/B comparison against the baseline at the same model/context/config

---

## Citation Index

| Paper | Venue | Year | arXiv | Public Code | Direct Relevance |
|-------|-------|------|-------|-------------|------------------|
| Fate | arXiv | 2025 | 2502.12224 | Third-party fork | High — PCIe 3.0 x16, Qwen3-30B-A3B |
| PreScope | arXiv | 2025 | 2509.23638 | None | High — Qwen3-30B-A3B, LLaPor predictor |
| HOBBIT | arXiv | 2024 | 2411.01433 | None (on llama.cpp) | High — llama.cpp, expert caching |
| SP-MoE | arXiv | 2025 | 2510.10302 | None | Medium — MTP + prefetching |
| Pre-gated MoE | ISCA | 2024 | — | None | Medium — pre-gating for prefetch |
| MoE-Infinity | arXiv | 2024 | 2401.14361 | None | Medium — expert tracing cache |
| MTP (Gloeckle) | ICML | 2024 | 2404.19737 | N/A | Already used via Unsloth |
| MoBiLE | arXiv | 2025 | 2510.12357 | None | Low — big-little expert strategy |
| MoQE | arXiv | 2024 | — | None | Low — asymmetric quant justification |
