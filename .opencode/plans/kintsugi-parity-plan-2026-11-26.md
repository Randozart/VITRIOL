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
