# LULL Phase 0 — Status Report

> **Date:** 2026-08-24 (session 09:00–15:00)
> **Worktree:** `/home/randozart/Desktop/Projects/VITRIOL-lull` (`lull-kv`, outer + submodule)
> **Status:** instrumentation VALIDATED; baseline sweep BLOCKED by pre-existing heap corruption in model-loading path (not LULL code)

---

## 1. What shipped (all committed)

| Commit | Content |
|---|---|
| `393730a` outer | Full LULL plan (`lull-plan-2026-08-24.md`) |
| `5968dc0f5` sub | `VITRIOL_LULL_PROFILE=1` split profiler (busy/idle CUDA events, VRAM watermark) |
| `7dfb21ea3` sub | Fault-tolerant event lifecycle (see §3.1) |
| `0c26dcc`…`df89340` outer | bench driver, analyzer, probes, fixes |

Files: `scripts/lull_bench.py` (driver), `scripts/lull_report.py` (analyzer),
`scripts/probe_corrupt.py` (corruption probe), `scripts/bisect_corrupt.sh`.

## 2. Instrumentation validation — PREMISE CONFIRMED

Clean telemetry captured (log `/tmp/opencode/lull-full-v2.log`, Q3_K_M c4096,
ts 27,9, MTP n1):

```
-- dev 0 (RTX 3060):   busy p50=56.9ms  idle p50=57.5ms
-- dev 1 (GTX 1070 Ti): busy p50=35.5ms  idle p50=97.7ms
```

**Each GPU idles roughly half the wall-clock during decode** — the lull resource
the LULL plan targets is real and large. dev1 reports double splits (two backend
instances share the device; state is keyed per instance, correct).

Decode t/s @c4096: rounds 13.2 / 11.7 / 8.0 → mean 10.95 (noisy; cold rounds).

## 3. Blocker: heap corruption (`free(): invalid pointer`)

### 3.1 Ruled out (each verified by controlled probe)

| Hypothesis | Verdict |
|---|---|
| LULL event instrumentation | Not the cause: crashes reproduce with `VITRIOL_LULL_PROFILE` unset; instrumentation made fault-tolerant anyway (self-disables on any CUDA error) |
| Query-before-record on events | Real secondary bug found & fixed early (caused downstream `invalid resource handle` kernel-launch failures); fixed in `7dfb21ea3` |
| MTP speculative decoding | Crashes with `--spec-type none` too |
| FlashAttention `-fa on/off` | Both crash |
| Tensor split vs single GPU | Both crash (with Q3_K_M) |
| Missing VITRIOL env (vs scripts/vitriol block) | Mirrored full env block — still crashes |
| git regression (bisect e659748e5 ↔ 3dc0454de) | Bisect invalid: "good" anchor ALSO corrupts when rebuilt fresh |

### 3.2 Isolation matrix (model file × flags)

| Model file | Flags | Result |
|---|---|---|
| `Qwen3.8-27B-Q3_K_M.gguf` (embedded MTP sibling) | any tested (±mtp/fa/ts/env) | **CORRUPT** at first-request teardown, every commit tested |
| `Qwen3.8-27B-UD-IQ2_S.gguf` (no embedded head) | plain, `-fa on`, single & dual GPU | **CLEAN**, two completions |
| same + `-ts 27,9` + q4 KV + cache flags + 7.7k prefill (bench) | | **CORRUPT** ← unresolved delta vs probe |
| `mtp-Qwen3.8-27B-Q4_0.gguf` | n/a | **is a standalone 18-tensor draft head, not a model** |

Crash signature: first `/completion` finishes (`print_timing` printed,
MTP stats if enabled), then `free(): invalid pointer` during slot/request
teardown. Deterministic enough to repro in <2 min at c2048–c8192.

### 3.3 Current suspicion

Q3_K_M's embedded-MTP-sibling load ("partial load — used 851/866 tensors",
override `qwen35→qwen35_mtp`, head loaded from same gguf) corrupts the heap;
UD-IQ2_S without a sibling is clean in the minimal probe. The remaining
probe-vs-bench delta (bench crashes even on UD-IQ2_S) points at interaction
between the deep-prefill/multi-request flow and one of {-ub 128, cache* off
flags, env block} — not yet isolated. Logs preserved under
`/tmp/opencode/probe-*.log`, `/tmp/opencode/kvq-*.log`, `/tmp/opencode/lull-*.log`.

**Recommended owner:** main-tree/Rebis side (model loading + server request
teardown are their active WIP area). The Q3_K_M file is the AGENTS.md-documented
default model — this bug likely affects any fresh build using it.

## 4. Baseline sweep status

BLOCKED pending §3.3 resolution. Resume procedure (once a stable config exists):

```bash
cd ~/Desktop/Projects/VITRIOL-lull
killall -9 llama-server
python3 scripts/lull_bench.py --ctx 8192  --tag c8k  --prefill 7680
python3 scripts/lull_bench.py --ctx 32768 --tag c32k --prefill 32256
python3 scripts/lull_bench.py --ctx 65536 --tag c64k --prefill 65024
python3 scripts/lull_bench.py --ctx 131072 --tag c131k --prefill 130560
for f in /tmp/opencode/lull-c*.log; do python3 scripts/lull_report.py $f; done
```

Note: server REJECTS prompts larger than ctx (HTTP 400) — filler sized at
5.2 chars/token with ctx-512 target; adjust if tokenizer differs.

## 5. Next actions

1. (User/Rebis agent) Adjudicate corruption: Q3_K_M embedded-sibling path.
2. (LULL) Finish baseline sweep → phase0 tables → gate Phases 1–4 as planned.
3. (LULL) Phase 1 attention-probe implementation can start in parallel —
   it is compile-time independent of the crashing runtime paths (new files +
   kv-cache score plumbing), gated behind `VITRIOL_KV_SCORE=off` default.

---

## Addendum — Phase 1 shipped (2026-08-24 18:xx)

**Attention-probe scoring implemented and verified end-to-end.**
Commit `46893d105` (submodule) / `5ef1e88` (outer).

Design as built (differs from plan §2.1 in one important way):
scoring ops are appended at the **tail of the decode graph** rather than
executed on a side stream — qwen35's custom graph builder asserts strict
last-node-expansion ordering, so mid-build insertion aborts
(`ggml_build_forward_impl` last==tensor). Tail placement keeps the sched's
automatic per-device routing and costs nothing extra; the "lull" execution
window moves to Phase 3 as planned.

Chain verified on UD-IQ2_S @ c2048 ts 27,9:
capture(il=3.., q=[256,24,1]) → tail subgraph → 16 layers scored per step
→ D2H at decode end → cells.score_set(decay·old+new). Marker line
`VITRIOL_KV_SCORE: probe active` fires once; default-off identical to
baseline; zero crashes.

Bugs found & fixed during bring-up (all in new code):
1. mid-build insertion vs builder invariant (→ tail append),
2. trailing prompt-chunk ubatch resetting a marked output (→ one-shot disarm),
3. context/cache split-brain: mark_output missing pending=true.

Remaining for the quality gate (§3 Phase 2): perplexity comparison of
score-driven vs blind eviction at depth — needs deep-prefill flows,
i.e. resolution of the §3 corruption (Q3_K_M embedded-sibling path).

## Addendum 2 — corruption SOLVED; deep-context unblocked (2026-08-24 evening)

**The heap corruption was ours, not the base's.** Bisect-by-flag isolated it:
`--ctx-checkpoints 0` alone corrupts the heap on multi-chunk prefill
(checkpoint code mishandles disabled=0); `--cache-ram 0` separately prevents
server readiness. Both flags came from MY bench hardening, which is why the
crash shadowed every commit/model/flag combination tested. Q3_K_M's embedded
sibling is exonerated as the primary cause (it may still have independent
quirks — retest once).

With bounded checkpointing (`--ctx-checkpoints 4 --checkpoint-every-n-tokens
8192`) and default cache-ram:

- Chunked fill to **7690 tokens @ c8192**: clean, probe scoring active,
  decode 11.8 t/s at depth (`scripts/lull_fill.py`, tag fill7680s).
- Phase 2 eviction rewrite shipped: global-lowest-score selection (old code
  stopped scanning at n_evict before sorting → index-order eviction),
  per-stream recent window, periodic VITRIOL_KV_EVICT telemetry.

**Open gap:** HTTP API pre-check rejects prompts ≥ ctx before init_batch,
so cache-full eviction can't be triggered by plain oversubscription;
needs context-shift interplay or multi-slot VRAM pressure to exercise at
runtime. Logic committed + reviewed; runtime gate deferred.
