# Tiered KV cache (`--kv-vram-cells`): port plan — 2026-11-25

Status: PLAN (research complete, not implemented)
Parent: `kintsugi-vitriol-port-2026-11-24.md` §4c, items D4g-target-1 and the tiered-KV decision.

## 1. Why this feature

Our engine serves Kintsugi-27B natively, but every limit that remains is a memory-placement limit,
not a compute limit:

| configuration | cards | ctx | measured |
|---|---|---|---|
| `kintsugi-27b` | 1 | 131072 (`tq3_0`) | 10,336 MiB, 8.80 t/s |
| `kintsugi-27b-262k` | 2 | 262144 (`q4_0`) | 7,662 + 7,302 MiB, 5.35 t/s (6.76 with DFlash2) |
| single card, 262144 | 1 | — | **cannot load**: ~5 GB of KV + 8.6 GB weights > 11.5 GB usable |
| single card + DFlash2 | 1 | 8192 | ~1.3 GB short at the tightest settings |

Their engine runs `-c 262144` on one card because of three features we do not have; this plan ports
the first and largest. The owner's stated preference is single-card operation with two-card available
as an option, so this is the feature that matches the goal.

**What it does not do**: it does not raise tokens/s. At depth it *lowers* it (§7). Any claim that this
closes the 1.66x speed gap is wrong — that is D4g, a separate item.

## 2. What their implementation actually is (read from source, not inferred)

Their engine source is on disk at `~/kintsugi/engine-src/kintsugi-engine-8f12baacf/` (MIT — `LICENSE`
is the ggml authors' MIT, compatible with our Apache-2.0 OR MIT). Facts, all with provenance:

**One VA range, page-granular physical backing** (`ggml/src/ggml-cuda/ggml-cuda.cu:1590`
`ggml_backend_cuda_tier_alloc`):

1. `cuMemAddressReserve(total)` reserves the whole K/V buffer as one device address range.
2. Each allocation granule is then backed either by VRAM (`cuMemCreate`, `CU_MEM_LOCATION_TYPE_DEVICE`)
   or by pinned system RAM (`ggml_cuda_hostmem_create` → `cuMemCreate` with
   `CU_MEM_LOCATION_TYPE_HOST_NUMA`/`HOST`, falling back across host types;
   `ggml-cuda.cu:1494-1590`), and mapped into place with `cuMemMap` + `cuMemSetAccess`.
3. Page → tier decision is `off - ip*part < vram_frac * part`: the *first* `vram_frac` of each part
   is VRAM, the rest is host. Parts = 2 (K, V in one buffer).
4. Because the range is device-addressable, **attention kernels need no change** — direct PCIe reads
   work by construction. Their code says so explicitly (`include/llama.h:399-402`).
5. Staging is a *performance* layer on top, not a correctness requirement: a second VA range aliases
   the same VRAM head pages, and each host run is replaced by a shared VRAM staging buffer
   (`ggml-cuda.cu:1273-1312`), so a DMA copy engine can stage host rows before the attention op
   (`ggml_cuda_tier_fa_begin/_end`, `ggml_cuda_tier_copy_out`), plus a `set_rows` write-back redirect
   for the KV writes that land past the line. Their launcher budgets the staging buffer at 1/16 of the
   full KV.

**Plumbing**: `llama_context_params.n_kv_vram_cells` (`include/llama.h:403`);
`llama_kv_cache` ctor param (`src/llama-kv-cache.h:118`); the buffer-type request is **24 lines**
inside the per-layer buffer selection (`src/llama-kv-cache.cpp:225-251`) which resolve
`"ggml_backend_cuda_tier_buffer_type"` through `ggml_backend_reg_get_proc_address` and fall back to
the ordinary device buffer when absent; registered at `ggml-cuda.cu:7642`.
Their certification is a self-describing test, `tests/test-kv-tier.cpp` (201 lines): one layer, K and V
in one buffer, q4_0, 10,240 cells, non-page-aligned so rows cross page boundaries, six batch steps that
place rows on both sides of each run boundary, comparing K bytes, V bytes and attention output against
an ordinary device buffer — and a documented bug history (write-back only used the first host run, so
V rows in the real host tail were never written; found precisely because K was not page-aligned).

**Auto-sizing** (`start-server.sh:219` `cells_for`): `budget = (ceiling - used - margin - fixed -
growth) - ctx*cell/16`, then `cells = budget*16/(cell*15)`, floored at 3328 cells, clamped to ctx. That
is: per-cell VRAM cost + 1/16 for staging; ceiling = min(97.7 % of card, card - driver-reserved - 8 MiB).

## 3. Our fork's gap — and the design decision it forces

The memory half ports cleanly. The attention half does not:

| file | ours | theirs |
|---|---|---|
| `ggml/src/ggml-cuda/ggml-cuda.cu` | 12 `cuMem*` call sites (pool only) | 54 (pool + host-location tier) |
| `ggml/src/ggml-cuda/fattn.cu` | 766 lines | **1714 lines** |
| `src/llama-kv-cache.cpp` | 3195 lines | 3076 lines |

Their tier hooks assume a **chunked attention loop** ours does not have: `ggml_cuda_fattn_chunk_cells()`,
`n_chunks`, a per-chunk `ggml_cuda_fattn_chunk` state, `meta` accumulator for the online merge, and one
DMA per chunk (`fattn.cu:1404-1560`). None of that exists in our `fattn.cu`. So a *full* port is their
chunked-FA rewrite plus the tier infrastructure.

**Decision: stage 1 ports the memory half only, with no staging alias and no `fattn.cu` change.**
Justification is point 2.4 above: the range is device-addressable, so kernels read host pages directly
and results are identical. Staging exists to make those reads faster, not possible. This shrinks the
port from "their FA rewrite + tier" to "tier buffer type + plumbing", and removes all risk to the
hottest kernel on a frozen fork (2,964 `FLASH_ATTN_EXT` cases stay untouched because the file is
untouched).

Stage 2 (staging alias + prefetch, or the chunked FA) is **conditional on measurement** (§5 T5).

## 4. Stages

- **T0 — feasibility probe (no repo change).** A ~60-line standalone C++ that `cuMemCreate`s host
  memory, maps it into a reserved VA range next to VRAM, writes from a CUDA kernel and reads back.
  Answers the only unproven environmental question: does driver 620.73 accept host location types here.
  If it refuses, the whole feature is dead on this host and we stop (their code anticipates this:
  "update the NVIDIA driver to 580 or newer").
- **T1 — `ggml_backend_cuda_tier_buffer_type`** in `ggml-cuda.cu`: host-memory helper
  (`ggml_cuda_hostmem_create` + a small pinned pool), `tier_alloc` (VA reserve, per-granule backing,
  `cuMemMap`/`cuMemSetAccess`, undo path on failure), free path, buffer-type registration and the
  `ggml_backend_reg_get_proc_address` export. `vram_frac`, `n_parts`, `tag` as in theirs. Honor
  `GGML_CUDA_NO_VMM` by returning an ordinary device buffer. No staging.
- **T2 — plumbing**: `n_kv_vram_cells` in `llama.h` cparams → `llama-cparams.h` → `llama-context.cpp`
  → `llama_kv_cache` ctor, plus the 24-line hook at the buffer-selection site, plus `--kv-vram-cells`
  in `common/arg.cpp` and `tools/server`.
- **T3 — launcher**: `[kv] vram_cells` → `--kv-vram-cells`, `0` = flag omitted so argv stays
  byte-identical (parity), positive value rides the fingerprint as `kvc=N`. Auto-size follows their
  `cells_for` arithmetic but reuses our calibrator's per-card free-space verdict rather than a second
  copy of the formula.
- **T4 — certification**: port `tests/test-kv-tier.cpp` adapted to no-staging (it mostly is already;
  its steps already run without CUDA graphs), plus a negative control that asserts the ordinary buffer
  is returned when the tier type is unavailable, plus a `test-backend-ops` spot run to show FA is
  untouched (it must be: the file is unchanged).
- **T5 — measurement and stage-2 decision** (§5).

## 5. Measurement (the gate that decides whether stage 2 happens)

Single card, `c 262144`, `q4_0` KV, `kvc` = the largest head the card allows (~40k cells predicted by
their arithmetic at 8.6 GB fixed). Measure pp and decode at filled depths 8k / 64k / 200k, against the
resident two-card baseline (5.35 t/s / 6.76 with DFlash2).

Stage 2 is warranted only if stage-1 depth numbers are bad *and* the badness is load-inefficiency
rather than unavoidable traffic. Note the arithmetic trap: at 262k depth one token's attention reads
the whole ~5 GB KV tail; at PCIe ~12 GB/s that is ≥400 ms/token no matter which design we use, so
**~2.5 t/s at 262k depth is a floor imposed by physics, not by our port**. Staging can only recover
the gap between coalesced DMA bandwidth and direct-load bandwidth. If stage 1 lands within ~2x of that
floor, stage 2 buys little and the plan stops at stage 1.

## 6. Invariants

Each is a property, not a test name; each names the test that pins it.

- **I1 (parity).** With `[kv] vram_cells = 0` — the default — the launcher emits byte-identical argv
  and the fingerprint is unchanged, so every existing profile behaves exactly as today.
  *Pinned by*: `scripts/tests/fingerprint-parity.sh` (argv + fingerprint diff against `HEAD`), and
  `test-kv-tier` being the only new binary.
- **I2 (identity).** A tiered buffer returns the bytes of an ordinary device buffer for every write
  and read, including rows that cross page and run boundaries and K/V sharing a page.
  *Pinned by*: `tests/test-kv-tier.cpp` (port of theirs, six steps straddling run boundaries).
- **I3 (attention untouched).** Stage 1 does not modify `fattn.cu` or any attention kernel.
  *Pinned by*: `git diff --stat` gate on the PR (no `fattn*` files) plus
  `test-backend-ops -o FLASH_ATTN_EXT` = 2964/2964 unchanged.
- **I4 (graceful absence).** Without VMM, without the proc address, or on a non-CUDA device, the KV
  cache uses the ordinary buffer and loads exactly as today; the feature never turns a working load
  into a failure.
  *Pinned by*: negative controls in `test-kv-tier` (SKIP path) and a CPU-only load test.
- **I5 (no silent capacity fiction).** If the requested `vram_cells` exceeds what the card can hold,
  the launcher refuses with the computed maximum instead of launching into an OOM.
  *Pinned by*: a launcher test with a card-size fixture asserting exit != 0 and the maximum in the message.
- **I6 (per-device KV quant composes).** Our fork overrides `layer_type_k/v` per device *after* buft
  selection (`llama-kv-cache.cpp:288+`); the tier gate must compare the **effective** layer types, not
  the ctor arguments, or a per-device override would leave the tier buffer holding a type it was not
  sized for.
  *Pinned by*: a test that sets `VITRIOL_KV_QUANT_K_GPU0` and asserts the tier path is taken/refused
  consistently with the effective type.
- **I7 (layers stay put).** The tier restructures a layer's K/V allocation, never which device a layer
  is assigned to; `-ts` behaviour and `--plan` verdicts are unchanged.
  *Pinned by*: `vitriol serve --plan` output equality for an unchanged profile.
- **I8 (single stream only).** Tiering is refused when `n_stream != 1`, on MLA models, or when
  `n_embd_k_gqa != n_embd_v_gqa` or `type_k != type_v` — the same guards theirs applies.
  *Pinned by*: a load test with `parallel=2` asserting the ordinary buffer and a log line saying why.
- **I9 (leak-free).** Every failure path in `tier_alloc` releases the reserved VA range and every
  mapped handle before returning null (their `undo` lambda is the model).
  *Pinned by*: a fault-injection test that caps the host pool and asserts no VA/handle growth across
  N failed loads.

## 7. Risks

1. **Driver/hardware refusal of host location types** — resolved by T0 before any code lands.
2. **Direct PCIe loads in the attention kernel may be much slower than DMA** (uncoalesced sectors
   through the BAR). Mitigated by the §5 floor analysis; if it is worse than 2x the floor, stage 2
   becomes mandatory and the plan grows to include their chunked FA — a separate, larger decision.
3. **Two allocators in one address space**: our pool already reserves a large VA range
   (`CUDA_POOL_VMM_MAX_SIZE`); the tier reservation must not exhaust the 49-bit VA space on a
   262k-x4-GB request. Checked in T1 with a repeated load/free cycle.
4. **Frozen fork**: this is hand-adapted, not cherry-picked. Every ported block carries a
   `PROVENANCE: kintsugi-engine 8f12baacf (MIT)` header naming the source file, as the Escha port does.
5. **Prompt-cache interaction**: KV state snapshots (`--slot-save-path`) serialize the buffer; a
   tiered buffer's host pages must serialize identically or restart-resume breaks. Not yet traced —
   T2 must confirm `llama_state_seq_get/set` walks the buffer rather than assuming device-resident
   memory.

## 8. Non-goals

- Their `--kq-mask-packed` (16x smaller attention mask): orthogonal, and its win is at `-ub 1024`
  sizes we do not use.
- Their two-stage output head (`LLAMA_HEAD2`): a different memory problem (200k-row head), the
  natural follow-on if tiered KV lands.
- Anything that changes results: tiering is a placement change, and I2 exists to keep it one.

---

## 9. Executed: B0-B4 (2026-11-25)

### B0 - driver probe: PASS

`/tmp/b0_tier_probe.cu`, standalone, no repo change. Mirrors `ggml_cuda_hostmem_create`
ordering, reserves a 64 MiB VA range, backs 32 MiB with VRAM and 32 MiB with pinned host RAM,
maps both, `cuMemSetAccess`, then a kernel writes across the line and 4096 ints are verified
around it.

```
gpu: NVIDIA GeForce RTX 3060 | CUDA driver API 13.1 | host NUMA id 0
OK   CU_MEM_LOCATION_TYPE_HOST (64 MiB), host granularity 2048 KiB, vram granularity 2048 KiB
PASS: 4096 ints verified around the line (0 bad)
```

**The host location type is accepted directly** — the NUMA fallback is not needed on this driver.

### B1/B2 - tier buffer type and KV hook: shipped (`6f34dce`)

Implemented as planned (`ggml_backend_cuda_tier_alloc`, host-mem helper with HOST->HOST_NUMA
fallback, per-granule backing, fault-safe undo, proc-address export, `GGML_CUDA_NO_VMM` ->
NULL -> ordinary buffer). Two findings the plan did not anticipate:

1. **The tiered buft must be recognised as a CUDA buft.** `ggml_backend_buft_is_cuda` keys on the
   `get_name` *function pointer*, so a bespoke buft made every op unsupported on the KV tensors
   (`buffer ... that cannot run the operation (NONE)`). Fixed by sharing
   `ggml_backend_cuda_buffer_type_context` and its get_name, appending `vram_frac`/`n_parts` as
   defaulted trailing fields.
2. **A buft can end up with no tensors.** Moving each layer's K/V to its own tiered buffer leaves
   the ordinary CUDA context empty, and a zero-size CUDA allocation fails with "failed to allocate
   buffer for kv cache". The allocation loop now skips an empty context.

The hook compares the **effective** layer types, after the per-device `VITRIOL_KV_QUANT_*_GPU<n>`
overrides, which is what I6 asked for.

### B3 - launcher: shipped (`2c2e6e2`)

`[kv] vram_cells` -> `VITRIOL_KV_VRAM_CELLS`, exported once so all launch sites see it; `0`/unset
unsets the variable, so an unset profile emits byte-identical argv and an unchanged fingerprint
(I1 verified by dry-run: a profile without the key produces zero `kvc=`/env occurrences). A positive
value rides the fingerprint as `kvc=N`:

```
VITRIOL-FINGERPRINT model=Kintsugi-27B-12G.gguf ts=1 c=262144 kv=q4_0/q4_0 fa=on ub=32 ...
  spec=none:0 par=1 kvc=32768 ssp=1 pr=0 cram=2048 cxr=1 cis=0
```

### B4 - identity test: shipped, ALL OK (`2c2e6e2`)

`tests/test-kv-tier.cpp` (their file, provenance noted; the direct-PCIe implementation makes it the
stronger claim):

```
K rows that differ: 0 of 10240 (ok)
V rows that differ: 0 of 10240 (ok)
step 5 (cells 9984 to 10239): attention output equal
ALL OK
GGML_CUDA_NO_VMM=1 -> SKIP: no tiered buffer on this build or device (no VMM)
```

### The result: native 262k on ONE card

| configuration | cards | VRAM | decode | note |
|---|---|---|---|---|
| `kintsugi-27b-tier-262k` (new) | **1** | 9,674 MiB dev0 | 8.39 t/s | native 262144, q4_0 KV, `vram_cells 32768` |
| head 2048, 3869-token prompt | 1 | 9,860 MiB | 7.63 t/s | 1,821 cells read from host RAM |
| same, resident (head 32768) | 1 | 10,334 MiB | 8.09 t/s | all in VRAM |

Needle exact through the host tail (`7-QX-4419`) at every head size. pp cost of the host tail at
`-ub 128`: 247.8 vs 250.1 t/s (**-1%**); decode -6%. Loads through the launcher in 21 s.

**This is the first time the 262k window runs on a single card on our engine**, which was the whole
point of track B. Their chunked-FA staging is therefore **not** needed for the capacity win, and
stage 2 stays unbuilt.

### Still blocked, and now precisely: DFlash2 on one card

Tiering KV did not unblock it, because KV was never the binding constraint:

```
E ggml_gallocr_reserve_n_impl: failed to allocate CUDA0 buffer of size 5564565504   (-ub 128)
E ggml_gallocr_reserve_n_impl: failed to allocate CUDA0 buffer of size 5442884096   (-ub 32)
```

A **~5.4-5.5 GB compute-buffer reserve that the draft context requests regardless of ubatch**, on a
card with ~11.5 GB free minus 8.6 GB of weights minus 1.1 GB of drafter. Their engine runs the same
drafter on one card with `fixed 9632` MiB, so their draft-context reserve is far smaller. **This is
the next concrete item for single-card dflash**, and it is unrelated to track B's mechanism.
