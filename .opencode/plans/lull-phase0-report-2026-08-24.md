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

## Addendum 3 — Phase 0 baselines COMPLETE (2026-08-24 evening)

Model UD-IQ2_S, ts 27,9, q4_0 KV, probe+sparse on, chunked fill
(`scripts/lull_fill.py`):

| depth | decode t/s (mean of 3×64) | notes |
|---|---|---|
| 7,690 | 11.78 | |
| 24,595 | 12.74 | + VITRIOL_LULL_PROFILE telemetry below |
| 31,780 | 11.72 | |
| 64,634 | 11.96 | |

Decode speed is flat vs context depth on this GQA+GDN-hybrid model
(16/65 full-attn layers): attention is a small share of per-token cost at
these depths. Implication for LULL: eviction's *speed* case lives at very
deep ctx (>100k) or comes from VRAM headroom (REBIS co-residency), not
from mid-depth masking gains.

Lull table @ 24,595 tokens (VITRIOL_LULL_PROFILE=1):

```
dev0 (3060): busy p50=36.6ms p95=273ms | idle p50=35.1ms  → ~50% wall idle
dev1 (1070Ti): busy p50=0.24ms (2nd instance micro-graphs); queue-backed
VRAM watermark @24k fill: dev0 free≈5.2GiB, dev1 free≈4.7GiB
```

Premise reconfirmed at scale: GPU0 idles about half of decode wall-clock.
Phase 0 gates SATISFIED; Phases 1–2 shipped; Phase 3–4 remain gated on
eviction runtime-trigger work (context-shift interplay or multi-slot).

## Addendum 4 — merged to main; multi-slot pressure findings (2026-08-24 night)

LULL merged into `main` (outer `03fa477`+`6f9d35e`) and submodule
`vitriol-mellum2` (`054fae712`, clean merge over LCP/MTP-head commits).
Main-tree `build/bin` rebuilt (61;86, PIC) and smoke-verified: probe marker
fires, generation coherent.

Quality gate slice 1 PASSED: greedy generation byte-identical probe-on vs
off at 12,090-token depth (scoring inert until eviction consumes scores —
correct no-regression behavior).

Dual-slot pressure test (--parallel 2 --kv-unified, two interleaved ~10k
sessions, sparse+probe): sessions grow cleanly to ~12k combined, then
server CANCELS incoming tasks at exhaustion WITHOUT invoking evict_sparse
— cancellation happens upstream of init_batch (prompt-cache restore path
and/or slot pre-check). No corruption; server survives. NEXT: trace exact
cancel site (update_slots slot-selection vs state_read_meta) and route
exhaustion through prepare()-with-eviction or fall back to shift.

## Addendum 5 — quant certification @131k (2026-08-24 night)

User question: can Qwen3.8-27B run at 3q/4q with reasonable speed/context?

Findings (UD-IQ2_S baseline: 11.7–12.7 t/s flat through 64k):

| config | result |
|---|---|
| Q3_K_M ts27,9 @131k | loads, OOM mid-fill ~17k (chunked) / ~37k (single-shot) |
| same + TQ3_0 KV (3.5bpw, −512MiB static) | identical wall ~17k |
| IQ3_S ts27,9 @131k (+probe+sparse, ±FA) | OOM/launch-fail at ~101–104k |
| IQ3_S / IQ2_S @65k | CLEAN: 9.3–12.0 t/s |

Key discoveries:
1. Fork supports TurboQuant KV: `--cache-type-k/v tq3_0` (3.5 bpw, 22%
   smaller than q4_0; tq3_1s 4.0bpw, tq3_4s also present) plus per-device
   asymmetric overrides (VITRIOL_KV_QUANT_GPU<d>). Works at runtime.
2. But static KV size is NOT the 131k blocker. Measured VRAM creep during
   chunked prefill: ~23 KiB/token on dev0 regardless of KV bits (VMM pool
   ratchet from varying graph shapes / scratch scaling with n_kv). Wall
   depth ∝ free margin: Q3_K_M (heaviest) dies earliest.
3. Practical ceiling today: **~64k certified; ~100k reachable (IQ3_S);
   131k blocked by creep**, not KV size.

Paths forward: LULL Phases 3–4 are the aligned fix (eviction/spill bounds
resident n_kv, which shrinks both creep exposure and scratch), or hunt the
per-token grower upstream (profiler watermark instrumentation already in
place to lead that hunt).

## Addendum 6 — "working before" reconciled; certification table (2026-08-24 night)

**Reproduction:** profile `qwen38-mtp-131k` on merged main build, faithful
env+flags (ctx 49152, MTP n1): **14.05 t/s** mean shallow-bench (ts 26,10;
historical 12.89 @ ts 27,9 — ts 27,9 no longer fits dev0 at context-init on
this base; rs-cache alloc OOM). No regression. The stale AGENTS.md row
("131072 / ~14.1") conflated window with filled depth: sweep benches used a
13-token prompt, and the profile's own meta documented "131K OOMs ~45-61K
tokens". Window ≠ usable depth.

**Certified depth×quant table** (single-shot mega-prefill + 3×64 decode,
full LULL substrate probe+sparse ON, tq3_0 KV, ts 26,10 ub64):

| quant | filled tokens | decode t/s | vs historical |
|---|---|---|---|
| Q3_K_M | 43,890 | 9.47 | at historical edge |
| Q3_K_M | **54,692** | **9.21** | **beats 45–61k death zone** |
| UD-IQ3_S | **96,836** | **11.32** | near prior 101k obs |
| UD-IQ2_S | 64,634 | 11.7–12.7 | prior addendum |

**NO_VMM experiment:** `GGML_CUDA_NO_VMM=1` did NOT lift the wall
(Q3_K_M@131k died at 59,392 tokens, launch-failure mode instead of OOM).
Creep grower still unidentified; profiler watermark hunt remains open.

**Practical answer to "3q/4q at reasonable speed/context":**
IQ3_S → ~97k ctx @ ~11 t/s. Q3_K_M → ~55k @ ~9 t/s. Both certified on the
merged main build. 4q: no local file; est. between the two.
