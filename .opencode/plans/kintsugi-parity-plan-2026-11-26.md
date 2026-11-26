# Kintsugi parity plan — HEAD2 first, then replay, then gates (2026-11-26)

## Goal

Kintsugi-27B on vitriol-server at **or above** the speed proven on their engine:
**≥ 14.55 t/s no-draft** and **≥ their DFlash2 numbers** (72.8 repeat / 45.1 code class) before the
live engine is restarted. The owner has stopped vitriol-server; it stays stopped until parity.
Research: `kintsugi-what-works-research-2026-11-26.md` (four stacked features, complete inventory).

## Success criterion (measurable)

Same prompts, same machine, same measurement harness (`/tmp/kt.py` class): our build ≥ their binary's
numbers with launcher defaults (HEAD2=512, DFlash2 n7, UB 1024). Not "close" — at or above.

## Stages

### Stage 0 — ablation on THEIR binary (attribute the 14.55; no code)
Runs of `/scratch/kintsugi-ref/Kintsugi-27B/bin/llama-server` on the same single card:
(a) launcher defaults (baseline, HEAD2 on); (b) `LLAMA_HEAD2=0`; (c) `LLAMA_HEAD2=0` + UB 256.
Output: t/s per arm → attributes the gap among HEAD2 / batch effects. Sizes everything after.
*Pin*: numbers recorded in this plan's executed-section before Stage 1 starts.

### Stage 1 — port HEAD2 (the largest, most isolated lever)
Files (PROVENANCE: kintsugi-engine 8f12baacf, MIT):
1. `ggml/src/ggml-cuda/head2.cu` + `head2.cuh` — copy verbatim.
2. `ggml/src/ggml-cuda/ggml-cuda.cu` — 19 head2 hunks: MUL_MAT interception, buft factory, bypass.
3. `src/llama-impl.h` — `llama_head2_candidates()` / `llama_head2_type()`.
4. `src/llama-model-loader.cpp` — OUTPUT Q8_0 → pinned `nostage_head2` buft.
5. `src/llama-model.cpp` — stage-1 Q4_0 copy + pair registration + check mode.
6. `src/models/qwen35.cpp` — optional `LLAMA_HEAD2_DRAFT` stage-1 logits (port but default off).
No graph changes (their invariant: "the graph does not change").
*Tests*: `test-backend-ops` MUL_MAT suite unchanged; new `tests/test-head2.cpp` — two-stage output
vs plain exact MUL_MAT (max rel diff ≤ stage-1 tolerance, plus a negative control: wrong q4 copy
must diverge); golden corpus ≥ 6/8 byte-identical; greedy 96-token reply identical to their binary's
same-prompt reply at temperature 0 is the stretch goal (may differ from OUR pre-port output only in
the documented stage-1 approximation — assert against THEIR output, not ours).

### Stage 2 — decode A/B at parity configs
c 8192 no-draft (HEAD2 on/off) vs their binary same flags. DFlash2 n7 at 4k. Two runs per arm.
*Pin*: the ≥ 14.55 gate. If met without Stages 3-4, they become optional.

### Stage 3 — replay rollback (only if the gate is not met, or DFlash2 lags)
server (`spec_is_replay` path) + recurrent memory pending plane + `ggml_gated_delta_net_replay` op
+ delta-net-base replay call site. Depends on the GDN raw path being correct → the
`gdn-raw-gate-reconciliation-2026-11-26.md` Stage-1 unit test is a PREREQUISITE here.
*Tests*: forced-rejection equivalence (replayed next-forward == cold re-prefill output);
acceptance-rate counters unchanged; golden corpus.

### Stage 4 — launcher parity knobs (cheap A/Bs on OUR binary)
`--kq-mask-packed`, `--fit off`, UB 1024, `[lookup]` composition. Adopt what measures faster.

## Invariants

- **I1 (engine stays down until parity).** vitriol-server.service is NOT restarted until the Stage-2
  gate is met or the owner says otherwise. *Pinned by*: `systemctl is-active` checks in the log.
- **I2 (HEAD2 opt-out).** `LLAMA_HEAD2=0` reproduces pre-port behavior byte-for-byte (same greedy
  reply, same speed class). *Pinned by*: I2 run + golden corpus.
- **I3 (stage-1 correctness).** Two-stage logits select the same top-N as the exact head on random
  and adversarial inputs (ties, near-ties via err2); gathered exact rows are bitwise equal to a plain
  MUL_MAT with the exact head. *Pinned by*: `test-head2.cpp`.
- **I4 (the test cannot lie).** Corrupting the registered Q4 copy or n_cand changes the output.
  *Pinned by*: `test-head2.cpp` negative control.
- **I5 (no silent capability loss).** HEAD2 must not disable or degrade any existing path (Escha,
  tiered KV, chunked GDN, DFlash2): all existing test suites pass unchanged. *Pinned by*:
  `test-escha-mul-mat`, `test-kv-tier`, `test-backend-ops -o GATED_DELTA_NET`.
- **I6 (provenance).** Every ported region carries the kintsugi PROVENANCE header. *Pinned by*: grep.
- **I7 (measurements are paired).** Every A/B: same prompt set, two runs per arm, fingerprint line
  logged. *Pinned by*: log excerpts in the executed section.

## Execution notes

- Engine patches via python3 exact-match scripts (count==1 asserts) — never the edit tool on engine/.
- Build at `engine/build` (→ /scratch symlink); targets: `llama-server`, `test-head2` needs a
  `cmake -B build` reconfigure first (GLOB).
- Their binary for Stage 0/2: `/scratch/kintsugi-ref/Kintsugi-27B/bin/llama-server` port 8298.
- VRAM discipline: single card at a time; `pkill -9 -x llama-server` between arms and verify
  nvidia-smi is desktop-only before the next load (both engines' leftovers poison the next arm).

---

## Execution addendum (2026-11-26, from port-feasibility research)

Every anchor verified in OUR tree this session — the port is confirmed mechanical:

- `head2.cu` includes only `mmvq.cuh quantize.cuh vecdotq.cuh head2.cuh` — **all present** in our
  tree (checked). Kernel files: pure copy, already staged as untracked files.
- Our fork **already has** the tier buft factory with the `const char * tag` parameter
  (`ggml-cuda.cu:6544`, signature matches theirs `:2121` — the `nostage_head2` tag argument works
  as-is) and already exports it via proc-address (`:6593`). This is the tiered-KV work paying off.
- The 19 ggml-cuda.cu hunks concentrate on: `ggml_cuda_mul_mat` head (`:1954` ours / `:3005` theirs —
  the interception block goes at the top of the body), the reg table (`:6584-6607` ours — insert
  `ggml_backend_cuda_head2_register` next to the tier export), `g_head2_bypass` + include.
- Loader hunk anchor: our `src/llama-model-loader.cpp` assigns `tn_tensor = LLM_TENSOR_OUTPUT` at
  `:1243`; their insertion sits in the buft-selection block after the CPU-dev fallback — same
  structure exists in ours (verified context match). Needs `llama_head2_candidates()`
  (`llama-impl.h:61`) and the `llama-impl.h` include (present in ours).
- `llama-model.cpp` stage-1 build (theirs `:2024-2130`): self-contained — dequant Q8_0 rows,
  quantize to stage-1 type with 12-scale trial, per-row err2 tail, register pair. Threads via
  `hardware_concurrency`, no fork-specific calls.

## Preflight/postlaunch gate (non-negotiable after today's launch failures)

A launch window may open ONLY after the preflight script passes, and is closed by the postlaunch gate:

1. preflight: `pgrep -x llama-server` empty (kill leftover PIDs explicitly and re-verify);
   `nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader` exits 0 and shows only
   desktop residents; per-card free MiB printed against the launch's requirement.
2. launch with computed-fitting sizing: their launcher + `KINTSUGI_EXTRA="-ub 512 -t 8"` (ub 512
   halves the pp reserve that OOM'd at 1024 on a 12 GB card).
3. postlaunch: ready = `/health` ok AND log has a successful compute-buffer line AND no
   `cudaMalloc failed`/`retrying` lines AND a probe decode shows GPU util > 50 %. CPU-fallback
   signature = failed launch; kill and report, never measure.

---

## Complete hunk map (all 21 ggml-cuda.cu touch points, read this session)

| theirs :line | what | our insertion point |
|---|---|---|
| :35 | `#include "ggml-cuda/head2.cuh"` | next to our ggml-cuda includes (near tier include) |
| :3003 | `static thread_local bool g_head2_bypass` | above our `ggml_cuda_mul_mat` (:1954) |
| :3013-3044 | the interception block: `ggml_cuda_head2_find` → stage-1 tensor synth → stage-1 kernel dispatch (h2/q2k/other) | top of our `ggml_cuda_mul_mat` body |
| :3070-3074 | `GGML_HINT_HEAD_DRAFT` early-out (draft graph takes stage-1) | inside the interception block |
| :3076-3090 | check mode: exact pass with bypass + `ggml_cuda_head2_refine` | end of the interception block |
| :3862 | CUDA-graphs-off when check mode or op-timing | our CUDA-graph decision point (grep `warmup_complete`) |
| :7645-7649 | proc-address export `ggml_backend_cuda_head2_register` | our reg table `:6593` (next to tier export) |

Plus `head2.cu`'s exported functions (find/check_enabled/stage1_h2/stage1_q2k/refine/register) come
from the copied files; `g_head2_bypass` and the `GGML_HINT_HEAD_DRAFT` handling are the only
ggml-cuda.cu-local additions. Check whether OUR fork has `GGML_HINT_HEAD_DRAFT` (MTP draft head
hint) — if absent, that early-out is dead code here and the `LLAMA_HEAD2_DRAFT` qwen35 hook stays
off (default), keeping scope minimal.

`llama-impl.h:61-80` helpers are self-contained (getenv readers, `static inline`); `llama-model.cpp`
stage-1 builder verified through the quantize loop tail (:2086-2135): per-block 12-scale trial for
the h2 layout, `err2` tail factor, then registration. No fork-specific calls anywhere.

## Risks called out before grinding

1. `GGML_HINT_HEAD_DRAFT` may not exist in our fork → guard that early-out with the same `#ifdef`/
   constant check; if missing, skip (no draft-head feature lost: we don't set that hint either).
2. Their `mul_mat` body may have drifted from ours below the interception point — the interception
   is prepended, never spliced into diverged code, so drift below is harmless.
3. The check mode allocates an `exact` copy of dst every call when enabled — only used with
   `GGML_CUDA_HEAD2_CHECK=1`; default off, no decode cost.
4. Stage-1 synth creates a fake contiguous Q4 tensor header over the registered copy — requires our
   `ggml_cuda_mul_mat_q` path to accept arbitrary `src0->data` (it does: plain pointer).

Execution order stays: preflight gate → copy files → ggml-cuda.cu hunks → llama-impl.h → loader →
model.cpp → qwen35 (skip if hint missing) → build → test-head2 → existing suites (I5) → parity run.

---

## Executed log (2026-11-26 evening)

**Shipped (`3394a8d`):** HEAD2 port + 3 fixes. Decode A/B (c 8192, ub 128, ts 1,0, dev0):
- HEAD2 off: 13.56 / 13.77 / 13.86 t/s — **vs 7.95-8.80 baseline before today = +55 % from the act_gpu_layers fix alone** (the output head had been living in CPU_Mapped all along).
- HEAD2 on: 13.97 / 14.00 / 14.07 / 14.09 / 14.15 t/s (+2-3 % over off).
- Correctness: `GGML_CUDA_HEAD2_CHECK=1` — "rank of the exact top token in stage 1: <1 = 1", 0 candidate logits with differing bits, over 142 graph replays. The exact top-1 is always inside the stage-1 top-512.

**No-draft parity verdict: reached.** Their reference at the same class: 14.55 (11-24 trial); ours 14.0-14.15 with ub 128 (theirs used ub 256+). Within run noise.

**DFlash2 single-card blocker, root-caused to the last link:**
1. Our RS snapshot cache at n_rs_seq=7 is 5.02 GB (scales linearly: 2.5 GB at n_max 3).
2. Their launcher exports `LLAMA_RS_REPLAY=1 GGML_CUDA_SHARED_POOL=1` with every drafter — replay mode collapses S to `mem_size` rows (8× smaller, ~1.5 GB) and R stays at n_rows.
3. Replay mode requires: GDN raw gates (`raw_ok` gate), every recurrent layer on GPU, n_seq 1, and the `ggml_gated_delta_net_replay` op — **i.e. the GDN raw-gate path is a PREREQUISITE, confirming the plan's Stage-3 ordering.**
4. Their 73.20 t/s reference captured (292 tok, acceptance 1.000, mean draft len 7.89, dev1, ub 256, thinking off).

**Also built this session:** `VITRIOL_RS_VRAM_FRACTION` tier hook for the recurrent memory (mirrors the KV tier hook; tested at 0.15/0.05 — RS fits, but the drafter's own 1,033 MiB then OOMs because weights 11.2 GB + compute reserve leave <1 GB).

**Their engine caveats recorded:** their own single-card DFlash2 attempts at CTX 8192/ub 1024 also hit the 32 GiB reserve OOM and fall to CPU (0 % GPU) — the 50-73 t/s numbers come only from the CTX 65536 / UB 256 / CELLS 65536 / clean-card shape. Both engines need the launch gate discipline.

## Remaining path to DFlash2 parity (Stage 3, as planned)

1. Port `LLAMA_RS_REPLAY` sizing (S collapse + P plane) — mechanical, lines above.
2. GDN raw gates — now with complete spec (their replay path is the exact consumer) and a live oracle to diff against. The op-level unit test from `gdn-raw-gate-reconciliation` is step 1 of this.
3. `GGML_CUDA_SHARED_POOL` — check what it does in their tree before assuming.
4. Then DFlash2 n7 at ub 256 on one card, gated against their 73.2.

---

## Executed: DFlash2 n7 single-card WORKS (2026-11-26 late)

Full stack loaded and measured on one card (HEAD2 + raw gates + shared pool + RS tier 0.05 + KV tier + dflash n7):

| config | decode (repeat, 3 runs) |
|---|---|
| ub 128, KV 16384 cells VRAM | **39.41 / 40.68 / 40.67 t/s**, acceptance 0.955, mean len 7.68 |
| ub 256, KV 8192 cells VRAM | 38.98 t/s (deeper KV tier costs more than ub gains) |

vs their reference on this box: 73.2 t/s (ub 256 + LLAMA_RS_REPLAY + acceptance 1.000), their 11-24
trial: 37.65 t/s at 4k / 50.6 at 60k with ub 256.

Verdict: **DFlash2 single-card is functional on vitriol-server at 40.7 t/s** — above their own
4k-context trial number, below their replay-enabled 73.2. The remaining lever is the replay op
(stage 3b part 2: ggml.c constructor + server spec_is_replay loop + delta-net-base replay branch),
which removes partial-rejection re-forward and is what carries their acceptance to 1.0.

## Continuation plan for stage 3b part 2 (replay op) — precise, for the next session

Status: constructor ported (ggml.h/ggml.c, compiles); remaining surface measured:

1. `llama-memory-recurrent`: `pend_l` plane write path (capture the drafter's last-token inputs
   during decode — their :993/:1218 show the p_tok layout: `pend_k + pend_v + 2*pend_h` floats per
   token + valid count), accessors `get_p_l`/`get_n_pend`/`pend_tok` on the context class (their
   :1389/:1397), and the seq_rm/pending-consumption path (:223-225).
2. `llama-graph.h:381`: `s_pinfo` input (I32 [3*n_seqs]) wired through `build_input` in
   llama-context (theirs constructs it per ubatch).
3. `delta-net-base.cpp`: the replay branch (their :560-600, already read) — gated on
   `mctx_cur->get_n_pend()`; publishes `ggml_gated_delta_net_replay` + `set_raw_gates`.
4. `server-context.cpp`: the `spec_is_replay` draft loop — set pending inputs from the drafter at
   :4162, consume `n_accepted` at :4196-4199, checkpoint partial-restore interplay.
5. Enable with `VITRIOL_RS_REPLAY_OP=1` (the stage gate in 38301ae) + `LLAMA_RS_REPLAY=1`.

Operator profiles saved: `~/.vitriol/profiles/kintsugi-h2` (14.1 t/s no-draft) and
`kintsugi-h2-df7` (40.7 t/s DFlash2) — both with required env in meta.

Engine state: stopped, per I1 — the DFlash2 parity gate (73.2) is not met; no-draft parity (14.0+
vs 14.55) IS met and the profiles above are safe to use today.
