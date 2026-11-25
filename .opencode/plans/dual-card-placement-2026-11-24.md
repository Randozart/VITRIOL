# Dual-card placement: Kintsugi live, permanent planner, Inkling 200k, trellis port

2026-11-24. Owner approved all four parts ("do it big").

## Background (measured, this session)

- Per-card free VRAM ~6.4 GB each after the Kintsugi engine (total 12 GB × 2
  is not combinable for one allocation; state *can* be split across cards).
- Kintsugi's launcher budgets the KV tier from `nvidia-smi | head -1` (dev0
  only) and floors at 3,328 cells → 98.7 % of its 262k KV was in host RAM,
  and `stop.sh` leaves the engine alive, polluting the next start's reading.
- Our stack has a pre-existing O(ctx) pp-buffer floor (~4.3 GiB at c 131k,
  ~35 KB per kv position, ub-independent, FA-independent) that blocks Inkling
  at c 204800; prime suspect is the DSpark layer-input taps
  (`t_layer_inp[il]`, `[n_embd, n_tokens]` F32 ≈ 1.2 GB per tap at c 131k).
- Kintsugi's GGUF declares `general.architecture = qwen35` but the tensors are
  EXL3 trellis types that only *their* engine decodes
  (`exl3` in their `libggml-cuda.so`, 0 matches in ours).

## Parts (in order; each ends with the 27B `vitriol-server` service restored
and fingerprint-verified — that is I1, checked after every engine swap)

### A — Kintsugi on both cards, today
- A1 Clean-start: kill stray `llama-server` PIDs; require
  `nvidia-smi --query-compute-apps` = desktop-only before start.
- A2 Start with `KINTSUGI_CELLS=98304 KINTSUGI_DRAFT=off KINTSUGI_UB=256
  KINTSUGI_CTX=65536`; gate: their log `cells 0..N in VRAM` with N ≫ 3328,
  load in minutes, `/health` ok.
- A3 **Both-cards gate**: VRAM grew on *both* cards (per-card
  `used_memory` for the engine PIDs), not just dev0. If the tier stays on
  dev0, add `KINTSUGI_EXTRA="-ts 6,5"` and re-gate.
- A4 Run probes P1–P5 (short / 4k / 64k / needle / identical-resend, thinking
  off), record t/s per depth vs their 3060 report.
- A5 `KINTSUGI_CTX=262144` full-window arm (same cells knob), then A/B the
  DFlash2 drafter on/off at 4k+64k (their claim: +25–35 %).
- A6 Restore `vitriol-server.service`; I1 fingerprint + `local-ok`.

### B — Permanent placement for any model on our stack
- B1 **Placement planner** in `scripts/vitriol` (`vitriol serve --plan`,
  default on at serve): read per-card free VRAM; size weights/KV/pp from the
  GGUF via `libvitriol` (per-tensor VRAM already there); emit
  `tensor_split` + `ubatch_size` + a fits/short warning; never rewrite a
  profile that already fits (I3).
- B2 **Kill the O(ctx) pp floor**: gallocr node dump on the Inkling target
  graph at c 131k to identify the consumer (DSpark tap hypothesis); gate the
  taps on drafter-enabled (I4: drafter runs byte-identical, target-only
  graphs lose the 1.2 GB/tap rows).

### C — Inkling 200 k proof
- C1 With B1+B2, `vitriol-server -c 204800` loads on the dual cards; 3.9 k
  context completion ≥ 12 t/s; the 4 full-attention layers' FA-bias path
  (e0ad53e) is exercised on the windowed layers.

### D — Big: EXL3 trellis port into our engine (Kintsugi under vitriol-server)
- D1 Port the trellis tensor types + CPU/CUDA kernels from their fork
  (b10858-8f12baacf) as individual patches with `PROVENANCE:` notes (Apache
  compat: their engine is llama.cpp-derived; keep attribution per the
  vendor-patch rule — this is a frozen-fork transplant, Inkling-style).
- D2 Kintsugi GGUF loads on `vitriol-server`; quality gate vs their
  engine's output (same prompts, temp 0, token-identical or KL ≤ 0.01 on a
  fixed seed — cross-engine reference, since we lack their golden set).
- D3 Then: Kintsugi as a Lapis-lane candidate profile with our KV caps,
  slots, ctx-resize.

## Invariants

- **I1 (parity of the daily driver)** — after EVERY engine swap,
  `vitriol-server.service` is active, the fingerprint line matches the 27B
  profile capture taken before the swap, and `local-ok` returns.
  *Pinned by* the gate script in `bench/fa-bias-gate.sh`-style: capture →
  swap → restart → diff.
- **I2 (Kintsugi quality)** — the probes pass: needle at 60k recalled
  exactly; the tool-calling shape is uncorrupted (P1/P5); decode t/s within
  30 % of their 3060 numbers (different clocks, so bounded not exact).
  *Pinned by* A4/A5 outputs in this plan's Results table.
- **I3 (planner non-intrusiveness)** — `--plan` on a config that already
  fits changes nothing (diff of emitted argv is empty). *Pinned by* dry-run
  on the 27B profile.
- **I4 (tap gating)** — with drafting enabled, the graph is
  byte-identical to today's drafter runs (their DFlash acceptance A/B
  unchanged); with drafting off, the gallocr dump shows the
  `t_layer_inp[il]` rows gone. *Pinned by* the dump + A/B.
- **I5 (Inkling 200 k)** — `vitriol-server -c 204800` loads; a 3.9 k
  completion runs ≥ 12 t/s. *Pinned by* C1.
- **I6 (trellis port correctness)** — Kintsugi on `vitriol-server` matches
  their engine on 20 fixed prompts at temp 0 (token-identical, or KL ≤
  0.01). *Pinned by* D2.
- **I7 (no silent fallback)** — each part that adds a kernel/type asserts
  support at graph build and fails loudly on CPU eviction (same doctrine as
  I3 of the FA-bias plan).

## Results so far (2026-11-24, single card, `KINTSUGI_CELLS=65536 CTX=65536 DRAFT=off UB=256`, 12.3 GB on dev0)

**Dual-GPU topology: BROKEN for Kintsugi.** The 2-card start loads fine but the
first request deadlocks: slot `processing` for 4+ min with 0 % util on both
cards, on chat *and* raw `/completion`. Single-card (`CUDA_VISIBLE_DEVICES=0`,
`-ts 1`) works immediately. Dual-card tiered KV is not the issue that Part A's
`KINTSUGI_CELLS` knob would fix — it is a request-time inter-device hang in
their build. That is what Part D (trellis + scheduler port into our engine)
exists for.

Load time single-card, warm cache, no drafting: **19 s**. (The 10.6 min
dual-card load was ~5 min weight+init + ~5 min draft-context build.)

| probe | result |
|---|---|
| P1 short (thinking off) | `"The capital of Italy, which hosts the 2026 Winter Olympics, is Rome."` — correct; decode **14.31 t/s**, prefill 52.1 t/s, n 20 |
| P2 4k summary | correct (identifies all 96 identical entries); prefill **325.2 t/s** @ 7,875 tok (int8 fast read), decode 14.06 |
| P4 needle @59,198 tok | exact recall: output `KINTSUGI-7-DELTA-42`; prefill **281.8 t/s**, 3m31s wall |
| P5 identical resend of P2 | full prefix-cache hit: `prompt_n 4`, `cached_tokens 7871`, byte-identical answer |

Their RTX 3060 report (MTP+lookup, thinking off): 41.5 t/s short / 33.2 @64k.
Our numbers are drafting-off, so decode is the non-speculated floor:
14.0–14.3 t/s. The A/B below measures the drafter gain on this box.

## Results (A5)

DFlash2 drafter A/B (single card, `KINTSUGI_DRAFT=dflash`):

| context | no drafting | DFlash2 | gain |
|---|---|---|---|
| 4k (P2) | 14.06 t/s | **37.65 t/s** | +168 % |
| 60k (P4) | 12.33 t/s | **50.6 t/s** | +311 % |

Their +25–35 % claim is against MTP+lookup, not against no-drafting — against
ours the drafter is the real win. Needle still exact with the drafter on.

Full 262,144 window loads in 15 s warm on one card: tier = 91,136 cells
(~1.7 GB) in VRAM, the rest in system RAM; P4 needle at 59,198 tok still
exact (`KINTSUGI-7-DELTA-42`), 281.5 t/s prefill, 12.26 t/s decode.

## I1 restore + Part B kickoff

| probe | Kintsugi @4k | @64k | their 3060 | Inkling 200k |
|---|---|---|---|---|
| decode | ? | ? | 41.5 / 33.2 t/s | ? (≥12 target) |
| needle | ? | ? | 24/24 @245k | — |
| cells in VRAM | ? | ? | 143,872 | — |


## Part B results (2026-11-24)

### B2: the O(ctx) pp floor is identified — it is not a single tensor

`VITRIOL_GRAPH_DUMP=N` (new opt-in switch, `llama-context.cpp` pp-reserve site)
names the graph. Top nodes at c 131072 / ub 32 (f16 KV):

```
256.0 MiB  VIEW/PERMUTE/SET_ROWS  cache_k/v_l{5,11,17,23,29,35,41,47}   [128,8,131072,1]
384.0 MiB  ADD  kq_b fold / dense chain  [131072,32,24,1]
11,981 nodes total
```

The floor is the **sum of ~8×256 MiB KV views + the dense per-head attention
chains** (4 full-attention layers × [ctx, ub, 24 heads] F32 kq + softmax).
The DSpark-tap hypothesis is ruled out: no `t_layer_inp` row appears anywhere
near the top. The ub-dependent 1.5 GB delta (ub 1→32) is exactly the
kq_b/kq/softmax chain on the 4 full-attention layers.

The **full-layer fold** (kq_b → F16 mask) was enabled for all layers
(`llama-graph.cpp`, supersedes the windowed-only gate): at 200k it is the only
thing that removes the 4×600 MiB dense nodes, and the pp reserve stays the
same byte size at c 131k (the buffer is sized from KV views + remaining dense
nodes, not from kq_b alone).

### The hard result: 200k at f16 KV is pool-bound, not buffer-bound

| config | outcome |
|---|---|
| c 204800, f16 KV, ts 1,1 | CUDA0 compute buffer 8.51 GB > dev0 free — fails (same 8513210112 B as pre-fa-bias-patch: the pool was always the wall) |
| c 204800, q4_0 KV, ts 1,1 | still fails (buffer unchanged; weights+KV+buffer > 2×12 GB) |
| c 131072, f16 KV, ts 1,1 | CUDA0 5.59 GB request fails |
| **c 131072, q4_0 KV, ts 1,1** | **loads in 18 s and serves** — prefill 88.2 t/s @ 21,485 tok, decode 4.35 t/s |

So the permanent solution for this box is: **q4_0 (or quant) KV is the
placement key, not the buffer**. 200k f16 needs ~15.6 GB/card minimum and
there is 11.5 GB — no ts or ub change can make that fit. I5's 200k sub-goal
is infeasible on this pool with the current engine; the *achievable* depth
point is 131k, and its decode speed (4.35 t/s at ~21k filled) is the next
target — that is the dense-attention decode cost on the 4 full layers, i.e.
the full-layer per-head FA mask work (C2/C3 of the FA-bias plan), not more
placement.

The I5 gate therefore stands as: *131k q4_0 loads and serves (meets)*;
*≥12 t/s at depth requires the FA work (open)*.
