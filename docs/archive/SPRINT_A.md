# Sprint A: TQ1_0 + Quick Wins

**Created:** 2026-05-18
**Status:** In Progress

**References:**
- `docs/VITRIOL_MASTER_PLAN.md` — Full optimization catalog
- `docs/CONFIG_REFERENCE.md` — Flag documentation
- `docs/TEST_REPORT_2026-05-17.md` — Previous test results
- `docs/PHASE3_OPTIMIZATIONS.md` — Phase 3 prior art
- `libvitriol/vitriol_shim.py` — Shim (for small bugfixes)

## Steps

### Step 1: Test TQ1_0 model load (10 sec)
```bash
./vitriol run --dry-run -m /home/randozart/Desktop/Projects/qwen3.6-35b-a3b-instruct-TQ1_0.gguf
```

### Step 2: Set parallel to 1 slot (10 sec)
Edit `~/.vitriol/config`: `parallel = 1`

### Step 3A: TQ1_0 with native CUDA (if load succeeds)
```bash
vitriol serve --detach -m /path/to/TQ1_0.gguf --engine-mode native --kv-quant q4_0 -c 24576
```

### Step 3B: TQ1_0 with CPU MoE fallback (if CUDA kernels missing)
```bash
vitriol serve --detach -m /path/to/TQ1_0.gguf --engine-mode native --kv-quant q4_0 --cpu-moe -t 2
```

### Step 4: Fix shim bugs (if memory mode needed later)
- Request context crash in `_store_turn`
- Tool call format normalization (missing `type: "function"`)

### Step 5: Test with OpenCode directly (no shim)

---

## Results

| Step | Status | Notes |
|------|--------|-------|
| 1 (TQ1_0 model load) | ✅ | Binary loads and recognizes TQ1_0 GGUF (`file type = TQ1_0 - 1.69 bpw ternary`) |
| 2 (parallel = 1) | ✅ | LRU thrash stopped (indirect — tok/s stable at 5.9) |
| 3A (TQ1_0 native CUDA) | ❌ | TQ2_0 CUDA kernels require deeper structural merge (template param mismatch, brace imbalance). Build restored to clean VITRIOL base. TQ integration deferred to a future sprint where we fork cleanly from turbo-tan. |
| 3B (Q2_K_XL + Q4_0 KV) | ✅ | KV cache 135 MiB (was 480 MiB). 5.9 tok/s gen, 118 tok/s eval. **The real win.** |
| 4 (shim bugs) | ⏳ | Deferred — not needed if running without shim |
| 5 (OpenCode direct) | ⏳ | Pending user test |

### Key Finding: TQ1_0 Needs CUDA Kernel Integration

The TQ1_0 ternary model loads but crashes with the VITRIOL buffer type (`--no-mmap` or `--cpu-moe` causes segfault). With plain `mmap` it runs entirely on CPU at 1.77 tok/s — too slow.

**The real win is Q4_0 KV cache quantization on the existing Q2_K_XL model.** The KV cache at 24K context dropped from 480 MiB to 135 MiB — a 3.6x improvement. At this density, 128K context would take ~700 MiB, easily fitting in 8 GB VRAM alongside the 1.3 GiB model and 512 MiB LRU cache.

### Next Steps for TQ1_0

To make TQ1_0 usable, the VITRIOL llama.cpp fork needs CUDA TQ1_0 kernels. These exist in PR #11183 (turbo-tan fork) but haven't been merged into the VITRIOL fork yet. Until then, the ternary model runs on CPU only.
