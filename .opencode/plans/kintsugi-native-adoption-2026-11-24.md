Status: **SUPERSEDED (same day).** The owner rejected Stage 1 in both its forms:
the objective is not to run their engine as a lane or inside our unit, it is to
make `vitriol-server` run the model. See
`kintsugi-vitriol-port-2026-11-24.md` for the live plan.

Kept because F1 (delta KV saves), F2 (`kq_mask_packed`) and F3 (`n_kv_vram_cells`)
are independently valuable findings about *their engine* that the replacement plan
deliberately does not fold in — they should be scheduled on their own merits, and
the measurements behind them stay cited here rather than being retyped.

---

# Kintsugi native adoption — findings + ladder (2026-11-24)

Status (original): **PLAN — awaiting confirmation.** Supersedes the "Part D" sketch in
`dual-card-placement-2026-11-24.md` (that document stays as the measurement
record; this is the adoption plan).

Owner direction: *"if it's a better invention, we shall adopt the invention
with provenance."*

---

## 0. What is actually established (measured, not assumed)

From `dual-card-placement-2026-11-24.md` Parts A–D and `kintsugi-trial-2026-11-24.md`:

| measurement | value | source |
|---|---|---|
| Kintsugi single-card load | 19 s warm | Part A |
| Kintsugi dual-card | **request-time deadlock** (slot stuck, 0 % both GPUs) | Part A |
| Kintsugi P1 decode | 14.31 t/s | Part A |
| Kintsugi P2 4k prefill / decode | 325.2 / 14.06 t/s | Part A |
| Kintsugi needle @59,198 tok | exact (`KINTSUGI-7-DELTA-42`), 281.8 t/s prefill | Part A |
| **Draft A/B (DFlash2)** | 4k 14.06 → **37.65 t/s (+168 %)**; 60k 12.33 → **50.6 t/s (+311 %)** | Part A |
| Kintsugi 262k hero mode | loads 15 s, 91,136 VRAM cells (~1.7 GB), needle exact @59k | Part A |
| Inkling 131k q4_0 (ours) | 18 s load, 88.2 t/s prefill @21,485, **4.35 t/s at depth** | Part C |
| Inkling 200k f16/q4_0 (ours) | **pool-bound**, CUDA0 request 8.51 GB, needs ~15.6 GB/card | Part C |
| Our O(ctx) pp floor | ~4.3 GiB ub-independent, = KV *views* + dense per-head chains on 4 full layers | Part B2 |
| Our 27B daily driver | 19.8 GB across both cards (ts 27,21: 9,718 + 10,076 MiB) | this session |
| Desktop GPU cost | kwin_wayland **26 MiB on card 0**; card 1 has no display duty | this session |

**Consequence:** running Kintsugi on card 1 is a *swap* of the 27B, not a
coexistence — the 27B is genuinely dual-card at 19.8 GB against ~11.5 GB free
per card.

---

## 1. Findings — five inventions, ranked by value ÷ cost

Their engine is a fork of llama.cpp via **PrismML-Eng/llama.cpp**, MIT
throughout (see §4). Five distinct things are separable. Only two of them are
about Kintsugi at all.

### F1 — Delta KV saves + background save jobs  ← *cheapest big win, not model-specific*

`src/llama-ext.h`:

```c
LLAMA_API size_t llama_state_seq_save_file_from(ctx, path, seq_id, pos_min, tokens, n);
LLAMA_API size_t llama_state_seq_save_size_from(ctx, seq_id, pos_min, n_tokens);
LLAMA_API size_t llama_state_seq_save_buffer_from(ctx, seq_id, pos_min, tokens, n, dst, size);
LLAMA_API struct llama_state_save_job * llama_state_seq_save_job_begin(ctx, seq_id, pos_min, tokens, n);
LLAMA_API size_t llama_state_seq_save_job_write(job, filepath);
LLAMA_API size_t llama_state_seq_save_job_host_bytes(job);
LLAMA_API size_t llama_state_seq_load_file_ext(ctx, path, dest_seq_id, n_tokens, flags);  // LLAMA_STATE_SEQ_FLAGS_APPEND = 4
```

A save writes **only the attention cells at or after `pos_min`** (recurrent
state in full); a load with `APPEND` adds them instead of replacing. The
`save_job` variant copies everything except K/V rows to host, **pins** the
sequence's cells (mutating ops wait), and a background thread writes the file
through a 64 MiB staging buffer.

**Why this matters to us specifically:** our KV cap breach incident
(2026-11-19) was rooted in `vitriol-checkpoint` writing a **full ~2.5 GiB slot
state per turn**, ~250 GiB over a long session, which then slipped past
`cache_max_gb` because the engine's disk scan only counted `cache-*`/`slot*`
prefixes. We fixed the *accounting*. Delta saves fix the *volume* — a per-turn
save becomes proportional to new tokens, not to window size. This is strictly
better than what we do and it is orthogonal to Kintsugi.

**Cost:** small. ~150 lines in `llama-context.cpp` + the ext API + append
semantics in `llama-kv-cache`. No new quant types, no kernels.
**Value:** high, and it lands on our *own* incident class.

### F2 — `kq_mask_packed` (1-bit FA mask, 16× smaller)  ← *directly attacks our pp floor*

`src/llama-cparams.h`:
```c
bool kq_mask_packed;   // FA KQ mask as packed bits (GGML_TYPE_I32, 32 cells per word): 16x less memory than f16
```

Gated in `llama-context.cpp:322` on `flash_attn && !use_alibi`; the fill lives
in `llama-kv-cache.cpp:2013` (`set_input_kq_mask_packed_impl<causal, swa, is_2d>`)
with the same predicate as the F16 fill minus the ALiBi value form. Called
from `set_input_kq_mask` at `:2168`; graph side at `llama-graph.cpp:41`
(`n_tokens/n_stream >= 32 && n_kv % 32 == 0`).

**Why this matters:** our C1/FA-bias finding was that folding the per-head bias
into an F16 mask frees **zero bytes** because the dense `kq`/softmax dominates.
A packed mask does not change `kq` — but at 204800 it takes the mask term from
~314 MB (24 heads × 204800 × 32 × 2 B) to ~19.6 MB. That is a 16× cut on one
of the components the fold was trying to shrink, and — more importantly — a
packed mask is *cheap to produce per head*, which is the missing half of C2/C3
(full-layer per-head FA at 200k was blocked by "expansion cost vs benefit").
It also composes with the mask channel we already shipped (`e0ad53e`).

**Cost:** medium. Mask fill (13 template-writing functions in their tree), a
new FA mask dtype path in CUDA, and it must coexist with `mask->ne[2] == n_head`.
**Value:** high — it is the most plausible route to Inkling at depth and to a
smaller pp reserve in general.

### F3 — `n_kv_vram_cells` (cell-granular KV tiering)  ← *the 262k-on-12GB trick*

`src/llama-cparams.h:21`:
```c
uint32_t n_kv_vram_cells;  // tiered KV: cells past this live in host memory (0 = all in device memory)
```
Plumbed at `llama-context.cpp:181` / defaulted `:3866`. Cells beyond the budget
live in host RAM; the launcher computes the budget from the model's fixed
footprint (`cells_for()`, `start-server.sh:219`).

**Why this matters:** *we already tier* — VRAM → host-RAM prompt cache →
Optane mirror. But ours is **slot-granular** (whole-KV snapshots); theirs is
**cell-granular**. Cell granularity is the difference between 262k and 131k on
one 12 GB card, and it is the piece with no upstream to port *from*.

**Cost:** high. Touches `llama-kv-cache`, the graph (views over a split
cache), and every backend's KV write path (a cell's location is a runtime
property).
**Value:** high but *speculative for us* — we run 27B and 31B models on dual
cards where the constraint is the pp reserve, not raw KV. Adopt only after F1/F2.

### F4 — Escha trellis codec (EXL3 GEMV/MoE)  ← *the Kintsugi enabler, and an ecosystem*

New ggml **ops**, not just types:
```c
ggml_escha_moe(...)      // MoE path, 8-of-128
ggml_escha_mul_mat(...)  // dense path (Qwen3.8-27B-Escha-W2: 400 projections)
// escha code tensor: [k/16][n/16][16K]; lut: [65536] f16 trellis codebook shared by all K
```
plus `GGML_TYPE_MS_*` ids 90/91/92 and `GGML_TYPE_PQ2_0`/`PTQ1_0` (142/143).

Port unit, measured:
| component | size |
|---|---|
| `ggml/src/ggml-cuda/exl3/` (vendored, has its own LICENSE) | **11 files, 2,470 lines** |
| `escha-gemv.cu` (+ .cuh) | 638 |
| `escha-moe.cu` (+ .cuh) | 1,704 |
| `ggml-cpu/mirai-s.cpp` + `mirai-s.cu` | 1,062 + … — **NOT NEEDED, see below** |
| plumbing (`ggml.c`, `ggml-quants.c`, `ggml-cpu/ops.cpp`, `ggml-cuda.cu`, `llama-arch/model/loader/graph`) | ~719 escha mentions |
| tests they ship | `test-escha-moe.cpp` (264) + `test-escha-mul-mat.cpp` (368) |

**Scope correction to my earlier estimate:** I said "3.4k kernel lines". The
honest figure is **~5.5–6.5k lines** including plumbing and the vendored EXL3
tree — but **Mirai S is out of scope**: their own release build sets
`-DGGML_CUDA_MIRAI=OFF` and the loader refuses Mirai S files. Kintsugi-27B is
Escha/W2. That removes ~1,500 lines and a whole codebook family.

**Value:** strategic — EXL3 is an ecosystem (ExLlamaV3-derived quants), so the
port makes `vitriol-server` loadable of a class of models, not one model.

### F5 — DFlash2 block drafter  ← *the speed number, and we have the chassis*

Their A/B is the headline: **12.33 → 50.6 t/s at 60k**. We already have
`dflash`/`dspark` in the fork (deltas from PR #32218 ported onto the inline
implementation) plus MTP and n-gram speculation. So this is a *port of a
drafter variant*, not a new subsystem — but it is only reachable after F4,
because the drafter is useless if the target model cannot load.

---

## 2. The ladder (each rung banks a win before the next rung spends)

### Stage 0 — write it down (this document). No code.

### Stage 1 — Kintsugi on card 1 as a **second local lane** (days)

Purpose: **buy the right to decide**. D is a week of kernels whose only payoff
is a speed number we can already measure for free. Stage 1 measures *answer
quality* on real work, and does it reversibly.

- `~/.vitriol/scripts/kintsugi-lane`: wrapper pinning `CUDA_VISIBLE_DEVICES=1`
  and setting `KINTSUGI_CELLS` explicitly (their launcher's `nvidia-smi | head
  -1` budgeting bug is invisible when only one device is visible — it
  self-heals, but we do not rely on that).
- Officina: a second local lane. `alias` stays `Lapis Occultus` for the
  `vitriol-server` lane (hard-coded at `tui/src/tui/heal.rs:586`,
  `mod.rs:4926`); the Kintsugi lane is a *separate* provider entry, resolved
  through `_shared/routing/apply.ts` (the only `pi.setModel` caller).
- Documented swap procedure for the 27B (`systemctl stop
  vitriol-server.service` → start Kintsugi lane → reverse).

**Gate G1:** Officina completes a real turn end-to-end on the Kintsugi lane;
27B restored and `local-ok` afterwards. **Decision point:** is the output good
enough to justify Stage 3?

### Stage 2 — adopt F1 then F2 (1–2 days each)

Deliberately ordered **before** Stage 3 and **independent of it**:

- **F1 first**: smallest diff, fixes our own KV-volume incident class, and its
  tests are pure state-comparison.
- **F2 second**: it is the plausible route to C2/C3 (Inkling depth), and it
  shrinks a term we have already measured. If F2 lands and C2/C3 then works,
  the pp-reserve story changes for every model we run, not just Inkling.

Both are MIT-derived llama.cpp work; both get `PROVENANCE:` headers.

**Gate G2:** Inkling at 131k q4_0 ≥ 8 t/s at depth (from 4.35), pp reserve
measurably smaller, and `test-backend-ops -o FLASH_ATTN_EXT` still 2964/2964.

### Stage 3 — F4 Escha port (multi-day, gated on G1)

Split so each piece is independently verifiable:

- **D1** type registration: `GGML_TYPE_ESCHA_*` in `ggml.h`/`ggml.c`,
  `ggml-quants.c` dequant reference (CPU, scalar, obviously-correct), `type_traits`,
  GGUF loader acceptance. Gate: a CPU-only dequant matches a fixture.
- **D2** CPU ops: `ggml_escha_mul_mat` + `ggml_escha_moe` on CPU.
  Gate: their shipped `test-escha-mul-mat` / `test-escha-moe` **ported and green**.
- **D3** CUDA: vendored `exl3/` + `escha-gemv.cu` + `escha-moe.cu`, with the
  cooperative-launch→plain-kernel adaptation documented (CUDA-graph-capturable,
  their comment at the top of `escha-gemv.cu` says exactly why).
- **D4** model path: `llama-arch`/`llama-model`/`llama-graph` wiring for the
  Escha projections; a `kintsugi-27b` profile.
- **D5** drafter: DFlash2 variant on top.

**Gate G3 (the one that matters):** 20 fixed prompts, **token-identical or
KL ≤ 0.01** against their engine. Without this we have ported a codec and
silently changed the model.

### Stage 4 — F3 cell-tiering (only if Stage 3 proves the model is worth it
and only if G2's numbers say the constraint is KV size rather than pp reserve).

### Explicit non-goals

- **Do not port Mirai S** — their own release disables it.
- **Do not port their dual-card scheduler** — their dual-GPU request path
  deadlocks; ours is the one that works.
- **Do not adopt their `nvidia-smi | head -1` VRAM budgeting** — that is the
  §"thrashing" defect (clamped 262144 → a 3,328-cell floor, 98.7 % of KV over
  PCIe every token).

---

## 3. Invariants

Numbered properties. Each must survive refactors of the code that implements it;
each names the test that pins it. A plan that cannot name its invariants is not
ready for implementation.

- **I1 — 27B daily driver is byte-identical.** No adoption touches behaviour at
  the live fingerprint `ts=27,21 c=262144 kv=q4_0/q4_0 fa=on ub=128` when the
  new switches are off. *Pinned by:* the existing golden decode matrix
  (`local-ok` fixture) re-run against the pre-change fingerprint.
- **I2 — `vitriol serve --plan` verdicts are unchanged.** The planner shipped in
  `e5c76f8` must still print `fits, no change needed` (exit 0) for the 27B config
  and `SHORT` (exit 2) for Inkling c 204800 f16. *Pinned by:* the two gate runs
  recorded in `dual-card-placement-2026-11-24.md` §B1, promoted to a test.
- **I3 — every new switch defaults OFF and is behaviour-neutral when off.** With
  `kq_mask_packed=false` and `n_kv_vram_cells=0`, decisions and pp reserve are
  **byte-identical** to today. *Pinned by:* pp-reserve comparison at c 131072
  (today: 5,587,683,072 B) with the switches on/off — the same method that
  refuted C1.
- **I4 — a delta save round-trips to the same KV state as a full save.** Chaining
  N delta saves then loading with `APPEND` yields a state buffer byte-equal to
  one full save at the final position (recurrent state compared in full).
  *Pinned by:* a state-comparison test over ≥3 chunks.
- **I5 — the KV disk cap holds with delta files.** `cache_max_gb` accounting
  still bounds total disk use when saves are delta-sized. *Pinned by:* the
  turn-file cap regression (`2d161df`) extended to delta files.
- **I6 — the per-head FA mask channel is preserved.** `e0ad53e`'s contract
  (`mask->ne[2] == n_head`, 128-dim heads, `ncols2 == 1`, all **three** launchers
  patched: main MMA ~1886, fixup ~1933, VEC ~114) still holds, and packed-mask
  mode composes with it. *Pinned by:* `test-backend-ops -o FLASH_ATTN_EXT`
  staying 2964/2964 across CUDA0/CUDA1/CPU, including the `mask_heads=-1`
  identical-slices diagnostic.
- **I7 — Escha parity.** Kintsugi on `vitriol-server` is token-identical or
  KL ≤ 0.01 vs their engine on 20 fixed prompts. *Pinned by:* the parity harness
  built in D4.
- **I8 — provenance is structural, not decorative.** Every adopted module carries
  `// PROVENANCE: inspiration — <repo> (<license>), what was learned, not copied`
  and retains the original copyright notice. *Pinned by:* the provenance lint
  (`I-P3`: unprovenanced items dropped at build time).

---

## 4. Provenance record (all MIT — compatible with our `Apache-2.0 OR MIT`)

| invention | origin | license | header to write |
|---|---|---|---|
| llama.cpp base | ggml-org | MIT | — |
| direct base | **PrismML-Eng/llama.cpp** | MIT | `inspiration — PrismML (MIT), base tree` |
| **Escha op (CPU + CUDA)** | **Ajay9o9 (AjHomelab), llama.cpp-escha** | MIT | `inspiration — Ajay9o9 llama.cpp-escha (MIT), Escha mul_mat/MoE op shape and CUDA tiling` |
| **EXL3 trellis GEMV** | **turboderp, exllamav3** | MIT (Copyright (c) 2025 Turboderp) | verbatim vendored — **keep `ggml/src/ggml-cuda/exl3/LICENSE`** |
| Mirai S | alesha-pro, llama.cpp-mirai-s | MIT | **not adopted** |
| `kq_mask_packed`, `n_kv_vram_cells`, delta saves | Kintsugi-27B engine (this fork) | MIT | `inspiration — Kintsugi-27B engine (MIT, llama.cpp-derived), <the idea>` |

Incoming-license check per `AGENTS.md`: MIT → "yes, with attribution retained".
Their engine keeps upstream MIT text and a `licenses/` + `vendor/` tree; the
vendored `exl3/LICENSE` must travel with the code.

---

## 5. Effort summary

| stage | content | estimate | risk |
|---|---|---|---|
| 1 | second local lane + swap procedure | ~0.5 day | low, fully reversible |
| 2a | F1 delta saves | ~1 day | low |
| 2b | F2 packed mask (+C2/C3 attempt) | ~1–2 days | medium (FA kernel surface) |
| 3 | F4 Escha D1–D5 (**without** Mirai) | ~4–6 days | medium-high (new ops, LUT codec) |
| 4 | F3 cell tiering | ~2–3 days | high (runtime cell placement) |

Sequencing is a value filter: Stage 1 costs half a day and can cancel Stage 3
(a week). Stage 2 is adopted *because* it is good, not because Kintsugi needs it.

---

## 6. Open question that Stage 1 answers and no kernel work can

Does Kintsugi-27B actually answer better than our 27B BuffedMod? Their published
AppWorld is 294/336 vs BF16 base 293/336 — +1 task, i.e. the fine-tune is
neutral-to-slightly-positive on that benchmark. That is a *thin* margin, and it
is the only reason to prefer it over EXL3-as-ecosystem. If Stage 1 says the
answers are not better, Stage 3 collapses to "port EXL3 for ecosystem access"
and should be scheduled on its own merits.

---

## Invariants (quick checklist for review)

I1 parity · I2 planner · I3 switches-off identity · I4 delta round-trip ·
I5 disk cap · I6 per-head FA mask · I7 Escha parity · I8 provenance lint
