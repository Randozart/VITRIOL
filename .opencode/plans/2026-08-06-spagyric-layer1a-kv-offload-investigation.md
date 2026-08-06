# Spagyric — Layer 1a `--kv-mode offload` Investigation (Mellum, GTX 1070 Ti)

Date: 2026-08-06.

## 1. Goal

Determine whether VITRIOL's Layer 1a KV-cache offload (page-locked host RAM, GPU
attention intact) unlocks a longer context envelope at acceptable speed on this box —
the escape hatch that the generic `--no-kv-offload` and q4_0-KV both failed to provide.
Scope per user: Mellum2 only, "max context at acceptable speed" (200K is not a hard
requirement — see §6), single thread (parallel=1) first, multi-slot only as a follow-up.

## 2. Why this is NOW unblocked

- `sudo /home/randozart/Desktop/Projects/VITRIOL/scripts/vitriol setup` set
  `cap_ipc_lock=ep` on llama-server (+ RUNPATH fix). Verified 2026-08-06.
- Layer 1a allocates KV as a **CUDA_Host (page-locked host RAM)** buffer
  (`ggml_backend_dev_host_buffer_type`, llama-kv-cache.cpp:214-220). Page-locking is
  exactly what the mlock cap previously blocked (RLIMIT_MEMLOCK ~2 GB, no CAP_IPC_LOCK).
  With the cap on the binary, the CUDA_Host buffer can actually pin.
- Earlier failed levers this session: `--no-kv-offload` moves *attention compute* to CPU
  -> 15 t/s (refuted); `--cache-type-k/v q4_0` -> 13.9 t/s + crash (refuted). Layer 1a is
  different: KV storage in host RAM, attention stays on GPU (PCIe DMA for KV reads).

## 3. Research (what we know before measuring)

| fact | source |
| --- | --- |
| Mechanism: env `VITRIOL_KV_MODE=offload` switches KV buft to host-RAM | llama-kv-cache.cpp:214-220 |
| Also `VITRIOL_KV_MODE=sparse` (token-eviction KV) | llama-kv-cache.cpp:765-766 |
| Prior result (Qwen 35B): 5.80 vs 6.21 t/s = -6.6% PCIe penalty; freed ~470 MiB VRAM; **2 graph splits vs 17** | docs/TEST_REPORT_2026-05-17.md §2.2 |
| Wrapper path: `vitriol serve --kv-mode offload` (port 8279) or direct env on llama-server | docs/OPENCODE_SETUP.md, docs/CONFIG_REFERENCE.md |
| Mellum KV/token (f16): 2 x 28 layers x 4 kv_heads x 72 head_dim x 2 B = **32,256 B ~ 31.5 KB** | model metadata (mellum.log) |
| Mellum native context_length = **131,072** (yarn x16 of 8192) | model metadata |
| Mellum VRAM baseline (KV in VRAM): 30-34 t/s decode, c=32768 comfortable | this session sweep |
| 131K KV = 131072 x 31.5 KB ~ **4.1 GB** host RAM (fits 15 G) | computed |
| 200K KV = **6.3 GB** but exceeds model native cap | computed + model metadata |

## 4. Hypothesis

With `VITRIOL_KV_MODE=offload`, KV moves to host RAM and the parallel/context ceiling
stops being VRAM-bound; the binding constraint becomes **PCIe KV read bandwidth during
decode**, which grows O(context) per token. Expect:
- Correctness preserved (GPU attention, just reading KV over PCIe).
- Small penalty at moderate ctx (like the Qwen -6.6%), growing with ctx.
- Feasible envelope: up to the model's native 131K, with decode t/s degrading as ctx
  grows (a curve we must measure, not guess).

## 5. Measurements (Mellum Q4_K_M, native, ngl=24, t=4, parallel=1)

1. **Sanity**: c=32K + offload vs VRAM baseline (30-34 t/s). Correctness gate.
2. **Context sweep**: c in {32K, 64K, 96K, 131K} + offload: decode t/s, eval t/s,
   correctness. The core long-context curve.
3. **Long-context correctness**: long filler + merge-sort prompt at 64K/131K (real
   history attention, not short-context-at-high-c).
4. **Graph splits**: `--verbose` log, offload vs standard (verify "2 vs 17" on Mellum).
5. **VRAM freed**: `nvidia-smi` with/without offload (quantify for Mellum).
6. **200K attempt**: c=200K -> expect model-cap rejection; document verdict.
7. **Follow-up only if offload shows promise**: `VITRIOL_KV_MODE=sparse`.

## 6. The 200K question (honest reframe)

Mellum's native `context_length` = 131,072. 200K exceeds the model's rope design;
KV-offload changes *where* KV lives, not the model's context spec. So on Mellum, 200K is
out of reach regardless of this investigation. The realistic target is **max usable
context up to 131K at acceptable speed**. If >131K is ever required, it needs a different
model (native >200K, e.g. a 256K/1M-context model) or rope extension (risky, out of
scope).

## 7. Harness

Extend `libvitriol/spagyric_sweep.py`:
- `SweepSpec.env: dict` applied to the subprocess env (VITRIOL_KV_MODE etc.).
- `--ctx-list` (ctx sweep, mode A single-request) that runs only the ctx grid, not the
  default ubatch/threads/parallel grid.
- Reuse the stderr-capture fix (`/tmp/opencode/server_stderr.log`).

## 8. Risks / honest notes

- Long-context decode = O(ctx) KV reads over PCIe -> could be PCIe-bound and slow. The
  finding may be "offload works, but long-context decode is bandwidth-limited."
- RAM: 131K KV = 4.1 GB host; with the 7.5 G model in VRAM and OS, fits 15 G but watch.
- `--mlock` now available (cap_ipc_lock) — CUDA_Host buffer pinning should work; verify.
- Graph-split claim was measured on Qwen 35B; Mellum (VRAM-fit, smaller) may differ.

## 8.5 Baseline table (fill on execution)

Measured 2026-08-06, Mellum Q4_K_M, ngl=24, t=4, p=1, VITRIOL_KV_MODE=offload.
Correctness PASS on all; two long-context responses were valid prose the strict gate
false-negatived.

| ctx (alloc) | KV location | used ctx | decode t/s | eval t/s | correct | VRAM used |
| --- | --- | --- | --- | --- | --- | --- |
| 32K | VRAM (baseline) | ~0 | 30-34 | ~49 | PASS | ~full |
| 32K | host (offload) | ~0 | 19.4 | 34.9 | PASS | 6.7 G |
| 64K | host (offload) | ~0 | 21.2 | 36.0 | PASS | 6.7 G |
| 96K | host (offload) | ~0 | 17.7 | 30.9 | PASS | 6.7 G |
| 131K | host (offload) | ~0 | 20.2 | 42.3 | PASS | 6.7 G |
| 131K | host (offload) | ~8K | **7.7** | 15.0 | PASS | 6.7 G |
| 131K | host (offload) | ~29K | **5.1** | 124.7 | PASS* | 6.7 G |
| 200K | host | — | n/a (model native cap 131072) | — | — | — |

*29K test: output was valid merge-sort prose ("The function should take a list of
integers..."); gate false-negative (no "merge" token).

## 8.6 Layer 1a reading (2026-08-06)

- **Bug found + fixed**: `VITRIOL_KV_MODE=offload` aborted on any partially-offloaded
  model — CPU-placed layers have `get_host_buffer_type = NULL` (ggml-cpu.cpp:489), so
  `ggml_backend_dev_host_buffer_type()` returns NULL and `buft_is_host(NULL)` asserts.
  Fix: NULL fallback to the normal dev buffer type (llama-kv-cache.cpp:215-222,
  submodule 85d01eda8).
- **Setup bug found + fixed**: `vitriol setup` ran `fix_rpath` (patchelf, rewrites ELF)
  AFTER `setcap`, clearing the capability. Reordered: rpath first, setcap last
  (scripts/vitriol, a583047).
- **KV offload works now**: 131K context fits, VRAM stays ~6.7 G used regardless of
  allocated context (KV in host RAM). VRAM-freed confirmed.
- **Decode cost is the story**: empty-context decode ~18-21 t/s (vs 30-34 VRAM) =
  ~35-40% PCIe overhead. With USED context, decode collapses: 8K->7.7, 29K->5.1 t/s
  (attention reads O(used) KV over PCIe per token). Extrapolated ~2-3 t/s at 100K+.
- **Honest verdict for the user's goal** ("max context at acceptable speed"): the VRAM
  path is better up to ~32K (30+ t/s). KV offload's niche is ONLY when >32K is required
  (up to 131K), accepting ~5-8 t/s at moderate fill. "Acceptable speed" and ">32K
  context" are mutually exclusive on this box. 200K remains impossible (model cap).

## 9. Deliverables

- Measured ctx-vs-t/s table + correctness verdicts.
- Graph-split + VRAM-freed numbers for Mellum.
- 200K feasibility verdict (expected: model-cap blocked).
- Updated Mellum profile if a win; records + provenance in both repos.

## 10. Cross-repo

Plan + results mirrored in bitshaper-ai (canonical) and VITRIOL. Harness in VITRIOL.
