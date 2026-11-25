# Kintsugi on vitriol-server — the Escha port (2026-11-24)

Status: **PLAN — awaiting confirmation.**
Supersedes `kintsugi-native-adoption-2026-11-24.md` (that document's Stage 1 —
"run their engine as a lane / inside our unit" — is **withdrawn**: the objective
is to make `vitriol-server` run the model, not to host their engine).

Owner direction: *"making VITRIOL compatible with Kintsugi"* — Kintsugi-27B
becomes **the** local model, served by **our** engine, on **our** unit, with
**our** tiers, governor, `/metrics`, and checkpoint extension. Their engine is
reference material only.

---

## 1. Why this is the right shape

| | their engine | vitriol-server + Escha |
|---|---|---|
| two cards | **request-time deadlock** (measured) | our 27B runs `ts 27,21` daily |
| placement choice | one card, because tiering is the only way it fits | **either card count, our call** (§2) |
| OOM shield | none (`nohup` + `start-server.sh`) | `OOMScoreAdjust=-500`, `Restart=always` |
| KV persistence | their Python layer + slot cache | `vitriol-checkpoint` + our turn-file cap |
| `/metrics` | stock | stock **+** `llamacpp:kv_ejected_total` |
| governor | n/a (404s on `/kv-policy`) | native |
| session/compaction | their saved-chat format | Officina's, unchanged |

The deadlock column is decisive. Their engine deadlocks on the first request with
a two-card layer split while both GPUs sit at 0 %. I recorded the cause as
tiered-KV inter-device synchronisation — it exists **only** because of their
cell-tiering, and §2 explains why we do not want that topology anyway. Remove
the tiering and the bug class has nowhere to live. Kintsugi on our engine should
be *more* robust than on theirs, because the model's arch (`qwen35`) is the one
our 27B already runs daily.

---

## 2. Placement: single card first, and why that is not a compromise

**The layer split buys capacity, not speed.** Per-token time with a contiguous
split is `Σ over cards (that card's share of the weights) / (that card's
bandwidth)` — which is the same wall-clock as one card streaming *all* the
weights at one card's bandwidth. The cards pipeline layers, they do not work in
parallel. Our own measurement says so: on Inkling the two GPUs never both
compute, each sits at 1–18 % utilisation and ~45 W of 170 W.

So the cross-card cost the owner is pointing at has two halves, and only one of
them is real:

| | cost | verdict |
|---|---|---|
| hidden-state hop between cards | `n_embd` × 4 B × ub. One hop per token at ub=1 ≈ **20 KB ≈ 1.7 µs** at ~12 GB/s, against ~70 ms/token | **negligible** — not the problem |
| the second card idling half the time | the split serialises; no bandwidth is gained | **real**, but it costs nothing in wall-clock — it just means two cards are not faster than one |

The conclusion is stronger than "less bus latency": **if the model fits on one
card, using two cards is pure waste — same speed, one card consumed.**

### Does it fit on one card?

Measured inputs:

- `FIXED` (weights + compute at load, from their own launcher's calibration for
  this exact file): **9,473 MiB = 9.25 GiB**. Note this is **0.57 GiB less than
  the 9.82 GiB file** — that part is not resident, which is why the single-card
  arithmetic is better than the file size suggests.
- `qwen35` KV at q4_0 (our measured figure for the same arch, ~17 of 65 layers
  attending): **19.1 KiB/token**; at q3_0, **~14.3 KiB/token**.
- usable VRAM per card: **~11.5 GiB**.

| window | KV @ q4_0 | fits on one card? | KV @ q3_0 | fits? |
|---|---|---|---|---|
| 32,768 | 0.60 GiB | yes | 0.45 GiB | yes |
| 65,536 | 1.19 GiB | yes | 0.89 GiB | yes |
| 98,304 | 1.79 GiB | yes (≈ 11.04 GiB total) | 1.34 GiB | yes |
| **131,072** | 2.39 GiB | **marginal** (~11.64) | **1.79 GiB** | **yes (~11.04)** |
| 204,800 | 3.73 GiB | no | 2.80 GiB | no |
| **262,144** | **4.78 GiB** | **no (≈ 14.2 GiB)** | 3.58 GiB | no |

So single-card reality is roughly **a 100k–140k window with the whole KV
resident**, and 262k does **not** fit.

### The fork this creates — and why "just use tiering" is the wrong answer

There are exactly three topologies, and one of them is the defect we already
diagnosed:

| topology | window | per-token KV over PCIe | cards consumed |
|---|---|---|---|
| **A. single card, resident KV** | ≤ ~140k (q3_0) | **zero** | 1 |
| **B. two cards, full window** | 262,144 | **zero** | 2 |
| **C. single card + their cell tiering** | 262,144 | **most of it** — this is the 3,328-cell-floor pathology | 1 |

The owner's instinct rules out C, because C is precisely *"bus KV between VRAM
and host every token"* — the same class of cost as bussing between cards, only
larger and with no second GPU to amortise it. A and B both have zero per-token
KV traffic; the only difference is cards consumed versus context length.

**So the plan targets A first, and treats B as the 262k fallback.** That is the
right default: it frees ~11.5 GB of a card at no measured speed cost, and it
deletes the whole class of multi-device bugs — including the deadlock their
engine suffers, which is a tiering bug (topology C) that A and B cannot express.

### The measurement that settles it

We do not have to take the physics on faith, and we already have the tool:
`scripts/vitriol serve --plan` prints per-card need vs free. Stage D5 runs the
same profile at `-ts 1` (single, card 1) and `-ts 27,21` (dual), same window,
same KV type, and records decode t/s at 4k and 60k plus prefill. **If single
card is not slower, topology A becomes the default and B becomes the 262k
profile.** If single card is measurably slower, I am wrong about the bandwidth
argument and B is the default — either way the answer is measured, not argued.

---

## 3. Measured port surface (dense-only)

Their tree is llama.cpp via PrismML-Eng, MIT throughout. **Two exclusions cut the
job roughly in half:**

- **`escha-moe.cu` (1,704 lines) not needed** — Kintsugi is **dense**. Verified:
  `_exps` tensor count in the GGUF = **0**, and their own header says
  `ggml_escha_mul_mat` is *"used by the dense escha checkpoints
  (Qwen3.8-27B-Escha-W2), where 400 projections"*.
- **`mirai-s.cu` / `ggml-cpu/mirai-s.cpp` not needed** — their own release
  builds `-DGGML_CUDA_MIRAI=OFF` and the loader refuses Mirai S files.

| component | files | size |
|---|---|---|
| vendored EXL3 GEMV | `ggml/src/ggml-cuda/exl3/` (11 files incl. `LICENSE`, `README.md`) | 2,470 L |
| Escha decode + fused swiglu | `escha-gemv.cu` (+`.cuh`) | 638 L |
| op definitions | `ggml.c`, `ggml-cuda/ggml-cuda.cu` | ~719 escha mentions across the plumbing set |
| type/loader/graph/model | `ggml.h`, `ggml-quants.c`, `llama-arch.*`, `llama-model-loader.cpp`, `llama-graph.*`, `src/models/qwen35.cpp` | see §4 |
| reference tests they ship | `tests/test-escha-mul-mat.cpp` | 368 L (port verbatim) |

### Escha is an **op triple**, not a quant type

This is the part that shapes the plan. Each projection is three tensors —
`escha_code`, `escha_rin`, `escha_rout` — plus shared tables `escha_lut`
(f16 `[65536]` trellis codebook), `escha_dep_k2`, `escha_dep_k3` (`i16 [16,256]`).
Graph side (`llama-graph.h`):

```c
struct llm_escha_exps   { … };   // code, rin, rout
struct llm_escha_tables { … };   // lut, dep_k2, dep_k3
struct llm_escha_moe    { … };   // gate/up/down, used by the dense path too
```

and `llama-graph.cpp` builds them with `build_escha_mm(tab, w, cur)` →
`ggml_escha_mul_mat(...)`.

### New ops to add (we add 2, not 4)

```
theirs - ours :  GGML_OP_ESCHA_MOE  GGML_OP_ESCHA_MUL_MAT  GGML_OP_MIRAI_MUL_MAT  GGML_OP_MIRAI_QUANTIZE
ours - theirs :  GGML_OP_TURBO_WHT
```

Adopt `GGML_OP_ESCHA_MUL_MAT` only. Skip `ESCHA_MOE`, both `MIRAI_*`. Note our
enum already diverges (`TURBO_WHT`) — **append at the end**, never insert; op
enums are internal and not serialised, so appending is safe.

### The loader already has what we need — this is the de-risking find

`llama-model-loader.cpp` needs a graph-**probe** case so the loader can discover
where each tensor is used before the graph exists:

```c
// theirs, :1006
case GGML_OP_ESCHA_MUL_MAT:  … builds a probe op, records code/rin/rout/lut/dep …
```

**Our frozen fork already has that machinery** — `src/llama-model-loader.cpp` is
1,857 lines vs their 1,844, and already carries 13 `case GGML_OP_*` arms
(`GET_ROWS`, `MUL_MAT`, `MUL_MAT_ID`, `ADD`, `ADD_ID`, `MUL`, `DIV`, `ROPE`,
`SSM_CONV`, `SSM_SCAN`, `SCALE`, …). We add **one case**. Their loader is
*slightly shorter* than ours — they did not fork it, they added arms. This was
my main unknown and it resolved favourably.

---

## 4. Stages and gates

### D0 — vendor the codec (no behaviour change)
Copy `ggml/src/ggml-cuda/exl3/` into `engine/ggml/src/ggml-cuda/exl3/`
**verbatim**, including `LICENSE` and `README.md`. Add it to the CUDA source
GLOB in the ggml-cuda `CMakeLists.txt`.
*Gate:* builds unchanged (nothing references it yet); `exl3/LICENSE` present.

### D1 — op registration + CPU reference
- `ggml.h`: append `GGML_OP_ESCHA_MUL_MAT`; declare `ggml_escha_mul_mat()`;
  add the trailing-scale/rotation doc comment from their header.
- `ggml.c`: op construction, `op_name`, `op_symbol`? (only if `GGML_OP_HAS_*`
  tables demand it), and the `ggml_compute_forward_*` dispatch.
- `ggml-quants.c`: a **scalar CPU reference** dequant (slow, obviously correct).
- `ggml-cpu/ops.cpp`: CPU `mul_mat` fallback.

*Gate:* our own fixture test — a synthetic Escha tensor decoded on the CPU matches
a reference decode computed in Python/NumPy from the published trellis rule
(`state = (state << T | symbol) & 0xFFFF`, codebook from `fmix32`). Nothing
downstream yet.

### D2 — CPU op correctness against their test
Port `tests/test-escha-mul-mat.cpp` **verbatim** (368 lines) into `engine/tests/`
and wire it into the test CMake.
*Gate:* their test passes on **CPU** against our reference implementation. This
is the strongest available check short of parity: the test was written against
their kernel, so a CPU pass means we read the format correctly.

### D3 — CUDA decode path
`escha-gemv.cu` (+`.cuh`) and the fused `escha_swiglu`, adapted the way their own
header comment requires: **exllamav3 runs the three stages in one cooperative
launch with `grid.sync`; a cooperative launch cannot be captured in a CUDA graph,
so the stages are plain kernels here.** Their code already made this adaptation —
we inherit it rather than re-derive it.
*Gate:* `test-escha-mul-mat` passes on CUDA0 and CUDA1 with the same values as
CPU; `test-backend-ops` still green except for cases we add.

### D4 — loader + arch + model wiring
- `llama-arch.cpp`: `LLM_KV_ESCHA_VERSION` (`%s.escha.version`),
  `LLM_TENSOR_ESCHA_LUT`, `LLM_TENSOR_ESCHA_DEP_K2/K3`, and the per-projection
  `escha_code`/`escha_rin`/`escha_rout` names.
- `llama-model-loader.cpp`: one probe case (see §3).
- `llama-graph.h/.cpp`: `llm_escha_exps`/`llm_escha_tables`/`llm_escha_moe` +
  `build_escha_mm`.
- **`src/models/qwen35.cpp`** (50 escha references in theirs) — the highest-risk
  file, because it is the file our **daily driver** uses. Every change is
  **arch-gated on `escha.version` being present**, the same technique we used for
  the Inkling arch correction. A model without that KV key must build the
  identical graph it builds today (I1).

*Gate:* the GGUF **loads** on `vitriol-server` and logs its arch + Escha version.

### D5 — profile + placement A/B + first token

Two profiles, one measurement:

- `kintsugi-27b` — **topology A**, single card (`CUDA_VISIBLE_DEVICES=1`, `-ts 1`),
  `c 131072`, **q3_0 KV** (the largest window whose whole KV is resident:
  ~11.04 GiB). `c 98304` q4_0 is the alternate if q3_0 quality disappoints.
- `kintsugi-27b-262k` — **topology B**, dual card (`ts` from the planner),
  `c 262144`, q4_0 KV — the long-context profile.

Both at MTP n=3 (native to the model), no external drafter yet.

*Gate:* **I7 parity** — 20 fixed prompts, token-identical or KL ≤ 0.01 against
their engine — **plus I10**, the placement A/B: single vs dual at equal window,
4k and 60k, decode + prefill. The A/B decides which profile is the default.

### D6 — drafter (later, separate)

Kintsugi carries MTP×3 + lookup, which we support natively — so some speculation
arrives free at D5. DFlash2 is a block-drafter variant and rides on the `dflash`
chassis already in our fork (PR #32218 deltas are in-tree). Strictly after I7.

---

## 4a. Stage results — D0..D3 shipped (2026-11-24)

| stage | commit | outcome |
|---|---|---|
| D0 vendor exl3 | `8504f7f` | 11 files / 2,470 lines verbatim, `LICENSE` + `README.md` (records upstream v1.6.7, commit `8c593e4e`) retained. Header-only, so no build output changed. |
| D1 register op + CPU ref | `02ba2f9` | `GGML_OP_ESCHA_MUL_MAT` appended (I4: `GGML_OP_COUNT` 102→103, `TURBO_WHT` keeps its value). Build EXIT=0. |
| D2 conformance on real data | `5225094` | `blk.0.attn_qkv`: CPU vs double reference **rmse/\|ref\|max 1.4e-07**. 3 negative controls fire. |
| D3 CUDA decode | `791cbc7` | CUDA0/CUDA1 **5.2e-05**, CPU 1.4e-07, all against the same reference. 8 checks, 0 failures. |

### Two scope corrections, measured rather than assumed

**1. "Skip `escha-moe.cu`" was wrong.** The plan's §3 assumed the dense path had
its own file. It does not: `ggml_cuda_op_escha_mul_mat` is *defined inside*
`escha-moe.cu` and calls `escha_rotate_in_dense`, `escha_matmul_dense`,
`escha_reconstruct_dense`, `escha_prefill_for`, `escha_finalize_dense` — all of
which live there. The two files are one codec. Both are taken **verbatim**; the
MoE entry point compiles but is unreachable because we never register
`GGML_OP_ESCHA_MOE`. Net effect on the estimate: +1,704 lines vs the plan, and a
matching −1 on the "do not port escha-moe" line in §8. Pruning the MoE kernels
later is a mechanical deletion; pruning them now would mean hand-editing a
1.7k-line kernel file during a first port.

**2. One newer-upstream symbol is absent from our frozen fork.**
`ggml_cuda_batch_invariant()` (batch-invariance toggle) does not exist in our
tree. The dense path uses it for one line — `n_rb_slices = gen &&
ggml_cuda_batch_invariant() ? 1 : n_rb` — so it is replaced by `n_rb_slices =
n_rb`, which is exactly the pre-feature behaviour. Marked inline at the call
site. We make no batch-invariance claim, and none of our outputs depend on it.

### What the data actually says about precision

CUDA is ~370× less precise than CPU (5.2e-05 vs 1.4e-07) because
`escha_rotate_in_dense<half>` quantises the rotated input to fp16 — by design,
since the tensor-core prefill path wants it in fp16 to move verbatim through
`cp.async`. CUDA0 and CUDA1 produce **identical** statistics, so there is no
per-device divergence to chase. The statistical error is well inside tolerance;
the binding precision gate is **I7 token parity at D5**, not D2/D3.

### Operational note for anyone touching `engine/` afterwards

The editor's auto-formatter rewrites an entire file when it touches one. On this
frozen vendored fork that produced a 9k-line formatting diff across five files
before the build even ran. The engine patches in this stage were therefore
applied through `scripts`-style exact-match replacements that assert a single
match per hunk, and the diffstays at **293 insertions / 2 deletions** for D1.
Do not open engine files in the editor for a one-line change.

## 5. Invariants

Each is a property, each names its pin.

- **I1 — the 27B daily driver is byte-identical.** Every `qwen35.cpp`/graph
  change is gated on `escha.version`; a model lacking it builds today's graph.
  *Pinned by:* the existing golden decode matrix re-run against the live
  fingerprint `ts=27,21 c=262144 kv=q4_0/q4_0 fa=on ub=128`.
- **I2 — `vitriol serve --plan` verdicts unchanged.** 27B → `fits, no change
  needed` exit 0; Inkling c 204800 f16 → `SHORT` exit 2. *Pinned by:* promoting
  the two recorded gate runs to a test.
- **I3 — the new op is unreachable unless a model asks for it.** With no Escha
  tensors present, decisions and pp reserve are byte-identical (c 131072 today =
  5,587,683,072 B). *Pinned by:* pp-reserve comparison, the method that refuted C1.
- **I4 — op enum append-only.** `GGML_OP_TURBO_WHT` and every existing id keep
  their values; new ops go at the end. *Pinned by:* a static assert on the enum
  count plus a compiled size-of-enum check.
- **I5 — CPU and CUDA agree.** `test-escha-mul-mat` passes on CPU, CUDA0, CUDA1
  with matching values. *Pinned by:* that test, three backends.
- **I6 — the per-head FA mask channel is preserved.** `e0ad53e`'s contract
  (`mask->ne[2] == n_head`, 128-dim heads, `ncols2 == 1`, all **three** launchers
  — main MMA ~1886, fixup ~1933, VEC ~114) still holds.
  *Pinned by:* `test-backend-ops -o FLASH_ATTN_EXT` = 2964/2964 on three backends.
- **I7 — Escha parity.** Kintsugi on `vitriol-server` is token-identical or
  KL ≤ 0.01 vs their engine on 20 fixed prompts. *Pinned by:* the D5 harness.
- **I8 — provenance is structural.** Adopted modules carry
  `// PROVENANCE: inspiration — <repo> (<license>), what was learned, not copied`;
  verbatim vendored code keeps its upstream copyright and `exl3/LICENSE`.
  *Pinned by:* the provenance lint (I-P3 drops unprovenanced items at build time).
- **I9 — the frozen fork stays frozen.** No upstream merges; every borrowed line
  is an individual patch with its provenance note (`docs/ENGINE_PROVENANCE.md`).
  *Pinned by:* a diff check that `engine/` contains no merge commits after the
  freeze commit.
- **I10 — the default placement is the faster one, measured.** Topology A (single
  card) is the default only if the D5 A/B shows it is not slower than topology B
  at equal window. *Pinned by:* the D5 A/B table (decode @4k/@60k, prefill,
  both topologies, same model and window), recorded in the profile's comment.
- **I11 — no per-token KV traffic across a bus in the default profile.** The
  default profile keeps **all** KV VRAM-resident on one card; `cache_ram` and
  disk tiers remain warm-resume mechanisms, never a per-token read path.
  *Pinned by:* the profile's computed KV footprint ≤ the single-card KV budget
  (asserted by `serve --plan`), plus a decode-rate check that the resident and
  tiered configurations are not within noise of each other.

---

## 6. Provenance record (all MIT — compatible with our `Apache-2.0 OR MIT`)

| origin | license | how it enters | header |
|---|---|---|---|
| ggml-org/llama.cpp | MIT | base | — |
| **PrismML-Eng/llama.cpp** | MIT | their direct base | `inspiration — PrismML (MIT)` |
| **Ajay9o9 (AjHomelab), llama.cpp-escha** | MIT | base of the Escha op, CPU + CUDA | `inspiration — Ajay9o9 llama.cpp-escha (MIT), Escha op shape and CUDA tiling` |
| **turboderp (exllamav3)** | MIT, © 2025 Turboderp | **verbatim vendored** `ggml-cuda/exl3/` | keep `exl3/LICENSE` untouched |
| Mirai S (alesha-pro) | MIT | **not adopted** | — |

Their own README states the release engine is built `-DGGML_CUDA_MIRAI=OFF`,
which is why the Mirai exclusions are principled rather than convenient.

---

## 7. Risks and how each is held

| risk | why it bites | mitigation |
|---|---|---|
| `qwen35.cpp` is the daily driver's file | one careless edit breaks the 27B silently | arch-gate on `escha.version`; I1 golden matrix runs before every commit |
| my single-card bandwidth argument is wrong | it would make topology A the slower default | I10: the D5 A/B decides it empirically; B is a first-class profile either way |
| 131k q3_0 does not actually fit | 11.04 GiB is arithmetic, not measured; pp reserve is the unknown term | `serve --plan --plan-pp-kb`, then D5 measures real VRAM before tuning; fall back to 98304 q4_0 |
| their tree is *much* newer llama.cpp (`kv-cache-dsa/msa/dsv4`) | graph-level API drift | hand-adapt per function, Inkling-transplant style; never cherry-pick |
| cooperative launch ↔ CUDA graphs | their fix is in-tree; re-deriving it is wasted work | copy the adaptation and its comment, cite it |
| codebook is a 65,536-entry f16 LUT | a wrong `dep` table silently degrades quality, not crashes | D2's ported test + I7 parity, not "it produced text" |
| 2.47 bpw quality | AppWorld 294/336 vs BF16 293/336 — a **+1 task** margin | I7 pins *our* decode against *theirs*; whether the model is worth keeping is a separate, later judgement |

---

## 8. What this plan does not do

- Does not run their engine, launcher, layer proxy, or slot cache — **at all**.
- Does not adopt `n_kv_vram_cells` cell tiering (§2: topology C puts most of the
  KV over PCIe every token; topologies A and B both avoid it).
- Does not adopt `kq_mask_packed` or delta KV saves **here** — both remain
  independently valuable and are recorded in the superseded document; they are
  not prerequisites for Kintsugi and should not block it.
- Does not adopt `nvidia-smi | head -1` VRAM budgeting (the defect that clamped
  262,144 → a 3,328-cell floor).
- Does not port `escha-moe`, `mirai-s`, `GGML_OP_MIRAI_*`, `GGML_TYPE_MS_*`,
  `PQ2_0`, `PTQ1_0`.

---

## 9. Effort

| stage | estimate | confidence |
|---|---|---|
| D0 vendor | 0.5 h | high |
| D1 registration + CPU reference | 1 day | high |
| D2 their test on CPU | 1 day | high — the test exists |
| D3 CUDA decode + swiglu | 1.5 days | medium |
| D4 loader/arch/graph/model | 1.5–2 days | medium — I1 is the constraint |
| D5 profile + parity | 1 day | medium — parity is the real bar |
| D6 drafter | later | low priority; MTP covers part of it |

**Order of proof:** their own test passing on CPU (D2) is the first moment we
know we can read the format. Everything before it is scaffolding; everything
after it is engineering.

---

## 4b. D4e–D5 results — Kintsugi runs on our engine (2026-11-25)

**Outcome: the coded checkpoint loads, serves and generates coherent text on
`vitriol-server`, on one card.** This is the plan's objective reached.

| measurement | value |
|---|---|
| load time (c 8192) | 3.5 s |
| weights resident | 8.6 GB on one RTX 3060 |
| profile `kintsugi-27b` (c 131072, tq3_0 KV, one card) | healthy in 8 s, 10.3 GB total |
| prefill | 65–80 t/s @ short prompt, **106 t/s at 30k depth** |
| decode | 8.86 t/s @ c 8192 / f16 KV; 7.1–7.5 t/s at 131072 / tq3_0 |
| coherence | correct arithmetic, correct noun recall at 30k depth |

### Four defects, each named by instrument rather than guessed

1. **Coded q/k/v had no matmul.** `build_qkv` calls `build_lora_mm` on
   `layer.wq/wk/wv`, and a coded layer has no dense weight → `ggml_mul_mat` on a
   null operand (the D4d SIGSEGV). `build_qkv` now takes an optional
   `llm_escha_tables*` and substitutes `build_escha_mm` per tensor. Substituting
   *there* rather than at the call site keeps bias/clamp/reshape in one place.
   **The gate is `llm_escha_exps::active()` — the code tensor's presence — not an
   arch flag**, so a plain checkpoint takes the identical path (I1/I3).

2. **The buft probe asked the wrong question.** Coded tensors are declared under
   their arch name (`attn_q.escha_code`), so llama.cpp infers `GGML_OP_MUL_MAT`
   and probes a matmul against an I16 weight. No backend accepts that, so the
   probe answered "no" and **9,819 MiB of a 10,546 MiB model landed in
   `CPU_Mapped`** while `-ngl 99` cheerfully reported "offloaded 66/66 layers".
   It looked like a slow model; it was a model running on the CPU. Fix:
   reclassify by element type before the probe. I16 is unambiguous; the fp16
   rotations are guarded by length (multiple of the 128-wide Hadamard block,
   and not the 65536-entry codebook).

3. **The rotation probe read `w` as both `rin` and `rout`.**
   `GGML_ASSERT(ggml_nelements(rout) == OC)` fired immediately, because IC == OC
   only by coincidence. `w` now stands in for `rin` alone and `rout` is mirrored
   to the same length — shape-valid for every projection in the format.

4. The probe context pinned at eight tensors left `ggml_new_tensor` returning
   null mid-probe.

### D4f: decode is correct but not yet fast

8.86 t/s against their engine's ~14.06 without a draft model. Measured, not
assumed:

- **No throttling**: 1747–1867 MHz, 104 W of a 120 W limit, no thermal/power brake.
- **GPU utilisation 43–75 %** → launch-bound, not bandwidth-bound, but only ~93 GB/s
  of the 360 GB/s peak is actually being used.
- `GGML_ESCHA_GEMV` sweep: **default/fused 8.86 > off 7.74 > wide 7.77 ≈ exl3 7.72
  ≈ narrow 7.71**. The shipped default is already the best of the four.
- `GGML_CUDA_USE_GRAPHS=1`: 8.64, i.e. no gain — the Escha node is evidently not
  graph-captured.
- **Decode is context-independent**: 8.84 t/s at c 512 vs 9.02 t/s at c 8192. So
  attention and KV traffic are not the limiter at all — it is pure weight
  streaming, 10.5 GB read per token at ~93 GB/s. That also means KV
  quantisation and sparse eviction cannot help decode on this model; only the
  streaming rate can.
- **RESOLVED by counting** (`VITRIOL_ESCHA_DEBUG=1`, commit `805187d`):
  `gemv=1600 other=0` — every decode projection goes through gemv, 400 per token
  in 123 ms. So the reconstruction fallback is *nearly as fast* as the specialised
  kernel, and both are limited by the same thing: streaming the code tensors.
  Turning gemv off costs 14 % because the alternative path is not the catastrophe
  the name suggests, not because gemv serves a subset.
- Effective read rate ~84–93 GB/s per 26 MB projection against a 360 GB/s card:
  **roughly 1.8x is on the table, but it is kernel-level work** (block sizing,
  cp.async pipelining of the code reads), not configuration and not placement.
- nsys could not attach on this host (no sudo for `--attach`; `nsys profile -p`
  is `--nvtx-capture`, not pid; launching under `nsys` ended with "Connection to
  Agent lost"), so there is **no kernel-level breakdown yet**. The gap is real and
  unexplained, not dismissed.

### D5: profiles shipped, I7 parity still open

- `kintsugi-27b` — one card, c 131072, `tq3_0/tq3_0` KV, slot dir
  `/scratch/kvtier-kintsugi`. **Loads and serves.**
- `kintsugi-27b-262k` — both cards, native c 262144, `q4_0/q4_0` KV, slot dir
  `/scratch/kvtier-kintsugi-262k`. **Load-tested: healthy in 12 s, 7,662 MiB on
  dev0 + 7,302 MiB on dev1, i.e. the native 262,144 window held across two cards.**
  This is the configuration their own engine deadlocks on (their launcher computes
  KV cells from `nvidia-smi | head -1` and clamps to a 3328-cell floor, pushing
  98.7 % of the KV over PCIe; the layer split then hangs on the first request).
  Two sequential requests completed normally here — no deadlock — with both cards
  live (16 % / 29 % utilisation).
  Decode **5.35 t/s vs 8.86 single-card**: layer split buys *capacity, not speed*,
  because the cards pipeline rather than compute in parallel. Exactly the I10
  expectation, now measured rather than predicted.
- **I7 parity is not established.** Their engine (`b10858-8f12baacf`) has never
  been built or run here, so the 20-prompt token-identical / KL ≤ 0.01 comparison
  has no reference side yet. What we do have is behavioural coherence at depth
  (30k prefill, correct recall) — necessary but not sufficient for I7.

### Invariant status

- **I1/I3 (plain models byte-identical)**: preserved by construction — the codec
  is gated on the code tensor's presence, and the 27B daily driver was restored
  and verified healthy after every load window this session.
- **I4 (op enum append-only)**: unchanged by D4e.
- **I7**: open, as above.

---

## 4c. I7 parity and the speed gap, measured against their release engine (2026-11-25)

Their engine never had to be built: `kintsugi-v1.1.6-linux.tar.gz` ships prebuilt
`bin/` (libggml-cuda.so + `llama-server`) and `start-server.sh`, extracted to
`/scratch/kintsugi-ref/`. So the reference is their actual release, not a
reconstruction.

### I7: 6 of 8 prompts byte-identical, greedy, 48 tokens

Same model file, c 8192, `-fa on`, no draft, temperature 0, `/completion`:

| prompt | longest common prefix | verdict |
|---|---|---|
| repeat, code, math, capital, list, prose | full length | **identical** |
| sort | 2 chars (diverges at token 3) | near-tie flip |
| unit | 6 chars (diverges at token 7) | near-tie flip |

Two flips out of eight, both at low-margin positions ("\n\n" then ```` ``` ```` vs
` thinking`; and an EOS call after "3500"). Nothing suggests a systematic numeric
error — and D2/D3 already pinned the op to 1.4e-07 against a double-precision
reference. **I7's stated bar (token-identical or KL <= 0.01) is met on the
identical six; the two flips still want a logprob comparison to close formally.**

### Speed, same host, same model, same context, same drafter

| | theirs | ours |
|---|---|---|
| no draft, single card | **14.55 t/s** (repeat) / 14.46 (code) | 8.80 / 8.74 |
| DFlash2 draft 3 (ours) / 7 (theirs) | **72.82** (repeat) / 45.11 (code), single card | 11.88, dual card |

**The 1.66x no-draft gap is not the gemv kernel** — theirs runs the same vendored
`exl3` kernels. It is also not draft quality: our acceptance is **1.00000 with mean
length 3.96**, i.e. every drafted token lands.

The difference is the **cost of verifying a small batch**. Ours emits ~4 tokens per
step at 11.88 t/s = ~296 ms/step, against 114 ms for a single-row token: verifying 4
rows costs **2.6x one row**. Theirs emits ~8 tokens per forward at 72.8 t/s, i.e. a
multi-row verification costs them **about one row**. Since decode is weight-streaming
bound, multi-row verification *should* be near-flat — the weights are read once for
the whole batch. Ours is not, and that is the whole gap.

Cause is localised to the `m = 2..8` path: `escha-moe.cu` sends `n_rows <=
ESCHA_GEN_MAX_ROWS (16)` to `ggml_cuda_escha_gemv`, which runs `mmode=1` with
`ROWS = EXL3_GEMV_MAX_M (8)` per chunk. Prefill (large m) is efficient — 270 t/s at
2048 rows — so the MMA tile is fine; it is the small-m chunk that is not.
Forcing the MMA path globally (`GGML_ESCHA_GEMV=0`) gave 10.03 t/s with acceptance
falling to 0.75 (the recon path changes the numerics), so that is not the fix as-is:
**D4g is now "make m = 2..8 verification cost about one row", not "improve the gemv".**

### DFlash2 on our engine: works, dual card only

- Dual card, c 8192, `--spec-type draft-dflash --spec-draft-n-max 3`: **11.88 t/s**
  (from 8.80 no-draft), and at 262k with q4_0 KV it is 6.76 vs 5.35 (+26 %).
- **Single card does not fit**: model 8.6 GB + draft 1.1 GB + recurrent state and
  compute buffers exceed 12 GB — consistently ~1.3 GB short at c 4096/`-ub 32`, and
  5.7 GB short at c 8192/n_max 7. Their single-card hero mode fits because of tiered
  KV (`--kv-vram-cells`), which we do not have. That is the concrete reason their
  dflash runs on one card and ours does not.
- Profile `kintsugi-27b-262k` now carries the drafter and loads through the launcher
  (`spec=draft-dflash:3`).

### Features their argv has that we do not

`--kv-vram-cells` (tiered VRAM/system-RAM KV), `--kq-mask-packed`, a two-stage output
head (`KINTSUGI_HEAD2`), and an int8 long-prompt read (`KINTSUGI_READ=fast`). The
first is what unblocks single-card dflash and the true 262k hero mode; it is the
natural next port after D4g.
