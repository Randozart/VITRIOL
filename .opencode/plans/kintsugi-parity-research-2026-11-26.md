# Kintsugi parity — research: what "working just like the original engine" actually means (2026-11-26)

Question: how do we make Kintsugi-27B run in vitriol-server at parity with their engine?
Prior rounds ported the GDN kernel (correct but slower) and the raw-gate graph half (garbage).
This round instead asked: **what features is their engine actually USING at runtime?** — because
their launcher (`/scratch/kintsugi-ref/Kintsugi-27B/start-server.sh`) is the manifest of the
fast configuration, and every feature it enables is a port item we never inventoried.

## 1. The feature manifest (from their launcher, the ground truth)

Their runtime configuration, decoded from `start-server.sh`:

| feature | flag / env | default | in our fork? |
|---|---|---|---|
| two-stage output head | `LLAMA_HEAD2=512`, `LLAMA_HEAD2_TYPE=q4_0` | **512 candidates** | **ABSENT** |
| DFlash2 drafter | `--spec-type draft-dflash --spec-draft-n-max 7` | draft 7 | present (`kintsugi-27b-262k`) |
| MTP fallback | `--spec-type ngram-mod,draft-mtp`, lookup 32, window 16384 | when no drafter | partial (`[lookup]` repaired `ba4c424`) |
| large ubatch | `UB=1024` (`pick UB 1024`, :317) | **1024** | works but untested at 1024 |
| tiered KV | `--kv-vram-cells $CELLS` | launcher-computed | **ported** (`kvc=`, `6f34dce`) |
| packed mask | `--kq-mask-packed` | always | **ABSENT** (flag not in our arg table) |
| no-fit pool | `--fit off` | always | flag absent |
| cache-ram | `--cache-ram 4096` | 4096 | present |
| reasoning budget | `--reasoning-budget 20480` … | medium thinking | flag absent |

**The single largest omission is HEAD2** — and it reframes the entire gap:

## 2. HEAD2 is the decode gap, most likely

Mechanism (`llama-model.cpp:2024`, `ggml-cuda.cu:3013-3060`, `head2.cu` 643 L + `head2.cuh` 59 L):

- the exact Q8_0 output head (6144 × ~205k vocab ≈ **1.2 GB of Q8_0**) is placed in **pinned system
  RAM** via a `nostage_head2` buffer type (loader `:1326-1337`), freeing ~600 MiB of VRAM
  (`HEAD2_MIB=606`, launcher :172-173);
- a **Q4_0 copy** lives in VRAM (stage 1); every decode MUL_MAT first runs the cheap copy
  (6144 × 205k in Q4_0 ≈ 600 MB read instead of 1.2 GB);
- a 4096-bin **radix-select kernel** picks the top-512 candidates per row
  (`HEAD2_BINS/HEAD2_SLACK`, head2.cu:9-10), then the exact Q8_0 rows are gathered for only those
  candidates (stage 2). Correct logits, ~half the head bandwidth, and the top-512 refines the draft
  path too (`LLAMA_HEAD2_DRAFT=1`: the MTP draft takes stage-1 logits only, qwen35.cpp:899-902).

The head matmul is THE dominant per-token cost on a 27B dense at batch 1 — our D4g decomposition
measured the step base at ~165 ms with the GDN rows ~27-51 ms. A 2× cheaper head is plausibly
worth more than everything we ported so far, and **it also explains why our single-card DFlash2
blocked**: 600 MiB of head freed is exactly the kind of headroom the 5.4 GB pp reserve needed.

Our D4g/D2 profile numbers never contained this because **our fork has no head2 at all** — we were
tuning a different engine.

## 3. Port surface of HEAD2 (complete inventory)

Files touched by HEAD2 in their tree (grep HEAD2|head2 across src/, common/, ggml-cuda/):

1. `ggml/src/ggml-cuda/head2.cu` (643 L) + `head2.cuh` (59 L) — radix select, stage-1 kernels,
   registration (`ggml_backend_cuda_head2_register(q8, q4, n_cand, q1_type, err2, n_rows)`,
   head2.cu:17). Self-contained; **no dependency on `ggml_cuda_batch_invariant`** (grep clean).
2. `ggml/src/ggml-cuda/ggml-cuda.cu` — **19 hunks**: the MUL_MAT interception (:3013+), the buft
   factory behind proc-address `ggml_backend_cuda_head2_register`, `g_head2_bypass` (:3003).
3. `src/llama-impl.h` — `llama_head2_candidates()` / `llama_head2_type()` env readers (:60-74).
4. `src/llama-model-loader.cpp` — intercept `LLM_TENSOR_OUTPUT` when Q8_0 2-D, stage it to a
   pinned-host `nostage_head2` buft (:1326-1337).
5. `src/llama-model.cpp` — build the Q4_0 stage-1 copy from the Q8_0 rows, register the pair,
   dequant-check (:2024-2060).
6. `src/models/qwen35.cpp` — draft graph takes stage-1 logits (optional, :899-902).

That is a **bounded, single-purpose port** — unlike the GDN kernel, which lives inside a graph of
interacting knobs. And it is testable in isolation: a unit test can build the pair, run both stages,
and compare against a plain MUL_MAT with the exact head (bitwise for Q8_0-gathered rows plus the
documented Q4 stage-1 tolerance).

## 4. Open questions for PLAN

- Q1: the loader calls the proc-address as `fn(atoi(dn+4), 0.0, 1, "nostage_head2")` — a **buft
  factory** signature, distinct from the tensor-pair register. PLAN must reconcile the two
  proc-address exports in ggml-cuda.cu's reg table (our fork's reg table layout may differ).
- Q2: does `--kq-mask-packed` / `--fit off` matter for us, or are they ancillary? (Mask packing
  affects FA memory; `fit off` prevents their auto pool shrink. Both cheap to test on THEIR binary
  first, by ablation.)
- Q3: is their 14.55 t/s measured with HEAD2 on? Their launcher defaults it ON (512). Our I7
  comparisons ran THEIR binary without any env — so **their measured advantage includes HEAD2**.
  This retroactively explains most of the gap arithmetic.
- Q4: ub 1024 on 12 GB cards — their launcher computes cells per card and defaults UB=1024 with
  `--fit off`; we never tested ub>128 for the dense head path. Cheap A/B on their binary.

## 5. Why this beats the GDN raw-gate plan as the NEXT move

- The GDN raw path is worth ~27-51 ms/row of a ~165 ms step at best, it failed parity once, and it
  interacts with our chunk kernel. HEAD2 is worth ~half the head bandwidth **on every token**, has a
  clean isolated test, and needs no graph change ("the graph does not change", llama-model.cpp:2029).
- The parity corpus (6/8 byte-identical) will validate HEAD2 exactly like it validated the codec:
  correctness gate first, then measure.
- Sequencing: HEAD2 first (big, isolated), then re-measure the gap; the GDN raw path only matters if
  a gap REMAINS after HEAD2 — and the same corpus decides.

## 6. Suggested stage shape (for PLAN)

0. Ablate on THEIR binary: run with `LLAMA_HEAD2=0` vs default. If 14.55 collapses toward ~9-10,
   HEAD2 is proven to be the gap — no more guessing. (Read-only, minutes, zero code.)
1. Port HEAD2 (files above) with PROVENANCE; unit test = two-stage vs exact MUL_MAT equivalence.
2. Golden corpus ≥ 6/8 + decode A/B at c 8192 and c 131072.
3. Only then revisit GDN raw gates with the remaining gap, if any.
