# LULL Certification & Depth Report — 2026-08-24

> **Branch:** `main` (outer `60499ac`, submodule merge `054fae712`)
> **Hardware:** RTX 3060 12 GiB (sm_86) + GTX 1070 Ti 8 GiB (sm_61), i7-3770
> **Model:** Qwen3.8-27B (qwen35 arch, GDN-hybrid: 16/65 full-attn layers, GQA 24/4, head_dim 256)

---

## 1. What this session established

### 1.1 Merge
LULL subsystem merged to `main` (outer `03fa477`+`6f9d35e`; submodule
`054fae712` over Rebis commits b78f27738/025291f6e — clean). Main-tree
build rebuilt (61;86, PIC), smoke-verified.

### 1.2 The "131k myth" reconciled
The historical "~14 t/s @ 131k" claim conflated **window** with **filled
depth**: sweep benches used a 13-token prompt, and profile
`qwen38-mtp-131k`'s own meta documented *"131K OOMs ~45-61K tokens on this
dual-GPU pair"* with actual window ctx=49152. Reproduction on merged build:

| config | shallow-bench t/s |
|---|---|
| ts 26,10, MTP n=1 (repro) | **14.05** |
| ts 26,10, MTP n=2 | 12.71 (depth-2 penalty confirmed) |
| ts 27,9 (profile original) | init-OOM on dev0 rs-cache — no longer initializes |

Profile fixed to `tensor_split = 26,10` (`60499ac`). Historical winner =
**ts 26,10 + depth-1 drafting**, exactly as the maintainer recalled
(the mtp=2 part of the recollection is contradicted by measurement).

### 1.3 Corruption saga (closed)
`--ctx-checkpoints 0` corrupts the heap (checkpoint code mishandles
disabled=0); `--cache-ram 0` blocks server readiness. Both were LULL-bench
hardening flags, not base regressions — which is why crashes shadowed every
commit/model combination during investigation. Q3_K_M's embedded-MTP sibling
exonerated as primary cause. Bounded values (`--ctx-checkpoints 4
--checkpoint-every-n-tokens 8192`) are mandatory; do not pass zeros.

### 1.4 TurboQuant KV discovered & adopted
Fork supports KV types beyond q4_0: **`tq3_0` = 3.5 bpw (−22% vs q4_0)**,
plus `tq3_1s` (4.0) / `tq3_4s`, with dedicated CUDA kernels
(`tq3-prefill.cuh`) and per-device asymmetric overrides
(`VITRIOL_KV_QUANT[_K|_V]_GPU<d>`). Works at runtime; static KV @131k drops
2304→1792 MiB.

### 1.5 NO_VMM experiment (negative)
`GGML_CUDA_NO_VMM=1` does not lift the depth wall (Q3_K_M@131k died at
59,392 tokens, launch-failure instead of OOM). The ~23 KiB/token dev0 VRAM
creep during prefill is NOT VMM-pool ratchet. Grower unidentified;
watermark instrumentation (`VITRIOL_LULL_PROFILE=1`) is in place for the hunt.

### 1.6 Quality gate slice 1 (passed)
Greedy generation byte-identical probe-on vs off at 12,090-token depth.
Scoring is inert until eviction consumes scores — correct no-regression.

---

## 2. Certified operating points

All: single-shot mega-prefill (shape-stable), 3×64-token greedy decode,
full LULL substrate (probe+sparse) except where noted, bounded checkpoints.

| quant | KV | ts | window | filled | decode t/s |
|---|---|---|---|---|---|
| UD-IQ2_S 7.8G | q4_0 | 27,9 | 65536 | 64,634 | 11.7–12.7 |
| UD-IQ3_S 11.2G | q4_0 | 26,10* | 65536 | 64,634 | 9.3–12.0 |
| **UD-IQ3_S** | **tq3_0** | **26,10** | 131072 | **96,836** | **11.32** |
| **Q3_K_M 12.9G** | **tq3_0** | **26,10** | 65536 | **54,692** | **9.21** |
| Q3_K_M | tq3_0 | 26,10 | 65536 | 43,890 | 9.47 |
| Q3_K_M | q4_0 | 26,10 | 131072 | 37k max (single-shot) | — |

\* one run at 27,9 also clean at this size.

Decode t/s is flat vs depth (8k→64k) — attention is a minor cost on this
hybrid arch at these depths. Eviction's payoff is capacity/VRAM headroom,
not mid-depth speed.

## 3. Known walls & mechanisms

| wall | evidence | status |
|---|---|---|
| dev0 VRAM creep ~23 KiB/token during prefill | nvidia-smi curve; independent of KV bits; kills deepest quant first (Q3_K_M ≪ IQ3_S ≪ IQ2_S margins) | OPEN — grower unknown; NO_VMM ruled out |
| eviction unreachable via HTTP | prompt-size pre-check + prompt-cache restore cancel before init_batch | OPEN — needs exhaustion routing through prepare() or multi-slot fix |
| Pascal FA/kernel limits ≥~100k n_kv | launch failures at 101–104k on two configs | OPEN — upstream-shaped |

## 4. Recommended configs (certified)

```bash
# quality-per-token sweet spot (~97k filled, ~11.3 t/s):
-ngl 99 -ts 26,10 --main-gpu 0 -ub 64 \
  --cache-type-k tq3_0 --cache-type-v tq3_0 -c 131072        # UD-IQ3_S

# max-compatibility Q3 (~55k filled, ~9.2 t/s): same flags, Q3_K_M gguf, -c 65536
```

MTP: depth-1 only; zero benefit measured on this HW; required for the 49k
window profile, drop for deep-window work if VRAM-tight.

## 5. Open threads (next sessions)

1. VRAM-creep grower hunt (watermark instrumentation ready).
2. Eviction runtime trigger (dual-slot cancellation path traced in Addendum 4).
3. Probe-quality ppl gate once eviction fires live.
4. Phases 3–4: lull scheduler + tiered cold-KV spill — the structural fix
   for the depth wall this report documents.

---

## 6. Depth-push results (2026-08-24 late session)

Goal: deepest possible FILLED context per quant; "window ≠ depth" discipline
(KV is allocated for the full window at load — right-size window ≈ target).

### New Q3_K_M record

| run | filled | decode t/s |
|---|---|---|
| ts 24,12, w98304, q4_0+FA, ub64 | **92,642** | **9.22** |
| same, w100354, target 96k | died 94,208 (dev0 OOM) | — |
| ts 26,10, w131072, tq3_0, substrate ON (earlier) | 54,692 | 9.21 |

Recipe that works: **right-sized window + `ts 24,12` + `-fa on` + q4_0 KV +
ub64**. Historical `ts 24,12` in AGENTS.md was likely exactly this deep-fill
configuration.

### Mechanism progress on the VRAM creep

1. Creep is dev0-only, linear (~23 KiB/token), independent of KV bits.
2. `VITRIOL_POOL_RESET=1` (new, committed): rewinds compute-pool bump
   allocator between graph evals → recovers ~20% depth (45k→67k on one
   config) by stopping the VMM LIFO-drift component.
3. Residual growth lives OUTSIDE the compute pool: per-graph-rebuild input
   buffers (kq masks etc.) go through direct cudaMalloc as n_kv grows.
   Next fix target: pool/reuse those input buffers or pad-and-reuse.
4. cuBLAS "failed to launch"/"unsupported value" failures at depth
   (dev1, tiny GEMMs) are secondary symptoms of device memory pressure /
   sticky state, not root causes.

### Updated practical envelope

| model | max certified fill | t/s | config |
|---|---|---|---|
| UD-IQ2_S | 64,634 | 11.7–12.7 | ts 27,9 q4_0 |
| Q3_K_M | **92,642** | **9.22** | ts 24,12 w98k q4_0 FA ub64 |
| UD-IQ3_S | **96,836** | **11.32** | ts 26,10 w131k tq3_0 |

ub32 note: dies with cublas invalid-parameter at ~53k (separate small bug,
avoid for now).

## 7. Final master configuration & speed investigation (2026-08-25)

### The tq3_0 KV discovery
Controlled matrix (MODE=off, ub64, FA, single-shot fills) isolated the day's
speed variance to **KV format**, not streaming/splits:

| config | decode |
|---|---|
| IQ2_S / IQ3_S, **q4_0 KV** — any split, any mode, any depth | **11.7–12.9 t/s** |
| IQ3_S, tq3_0 KV, resident | 6.86 ✗ (no MMQ path → slow dequant attention) |
| IQ3_S + separate MTP head | 8.45 and degrading ✗ |

Streaming exonerated as the decode villain at these sizes; the AGENTS.md
"always DMA" rule is formally rescinded (see protocol §1). Historical
golden-era profiles recovered from `~/.vitriol/profiles/`:
`qwen38-iq2s-100k` (**ts 1,0 — fully single-GPU**, ctx 100k!) and
`qwen38-iq3s-131k` (ts 70,30).

### Master profile final form (`qwen38-master`)
IQ3_S · q4_0 KV · ts 24,12 · ub64 · c131072 · MODE=off · -fa on ·
--no-mmap · ckpt 4×8192 · sparse+probe+pool-reset exports.
Certified: **92,642 tok filled @ ~9–12.4 t/s** (rounds noisy; best 12.4).
tq3_0+stream remains the stable-deep alternative (11.32 @ 96.8k) if a
session needs >92k.

### Infrastructure shipped this round
1. `VITRIOL-FINGERPRINT` at all three launch sites + runners (argv in every
   RESULT); silent flag drift now review-blockable.
2. `kv.checkpoint_every_n_tokens` profile key (was hardcoded 2048).
3. `lull_reuse_audit.py` — per-turn cache-hit/forced-miss accounting.
4. Launcher `model.alias` support (array-safe): Lapis Occultus lives.
5. hermes providers.custom timeouts (3600s/1800s) as prefill insurance.
6. Production relaunched detached under persistent logging; TUI TPS gauge
   revived (heartbeat file restored).

### Open
- Reuse audit accumulates with real hermes traffic; forced-full rate is the
  number to watch (verdict logic built into auditor).
- Decode plateau ~12 t/s appears hardware/kernel-equilibrium for this arch;
  CUDA graphs confirmed engaged during stable decode.
