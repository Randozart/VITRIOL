# REBIS — Phase 0/1 plan (dual-model drafter/verifier loop)

**Date:** 2026-08-21 17:40
**Status:** executing
**Concept:** Mellum2 (fast MoE drafter, pinned 1070 Ti) + Qwen3.8 (verifier/planner) in a
bounded recursive refinement loop ("poke-and-refine"), with async shadow prefill to hide
Qwen's slow ingestion. Working names: loop = **Rebis**, packet = **Mandatum**, shadow
prefill = **Anticipatio**.

## Hardware reality (measured/verified)

- RTX 3060 12 GB (sm_86) + GTX 1070 Ti 8 GB (sm_61), 20 GB total.
- Qwen3.8-27B IQ3_S = 12.0 GB weights → cannot fit 3060 alone; current profile streams
  across both GPUs (ts 24,12).
- Mellum2 = 12B MoE / 2.5B active, 28 layers, 4 KV heads (~57 KB/token fp16 KV), SWA 3:1,
  no exported MTP head.
- Strand-Rust-Coder-14B Q4_K_M (9 GB, dense) on disk as control drafter.

## P0.0 arch gate — PASSED 2026-08-21 17:41

- `LLM_ARCH_MELLUM`/"mellum" registered (llama.cpp/src/llama-arch.cpp:140).
- Full implementation: llama.cpp/src/models/mellum.cpp (`load_arch_hparams`,
  `load_arch_tensors`, `build_arch_graph`, MoE expert validation).
- Vocab pre-types "mellum"/"mellum2" (llama-vocab.cpp:1999,2165).
- MXFP4 CUDA kernels present (ggml-cuda convert/mmvq/mmq/quantize).

## Decisions (user-selected)

- Drafter flavor: stock Mellum2-Thinking (claude distill not essential).
- Primary quant: mradermacher i1-IQ4_XS 6.8 GB; fallback ladder MXFP4_MOE 7.0 → i1-IQ3_M 6.0.
- Qwen side: test both IQ3_S-streaming and UD-IQ2_S-resident in Phase 0.
- Controller: `libvitriol/rebis.py` (Python, HTTP-only, zero C++ changes).

## Phase 0 — measurement matrix

| # | config | placement | metrics |
|---|---|---|---|
| T0 | Qwen IQ3_S current profile | ts 24,12 stream | decode t/s, prefill @1k/4k/16k |
| T1 | Mellum i1-IQ4_XS ctx16k q4_0 KV | pinned 1070 Ti | decode t/s, VRAM peak |
| T2 | Qwen UD-IQ2_S ctx32k→64k q4_0 KV | pinned 3060 | decode t/s, VRAM peak |
| T3 | T1+T2 co-resident | both GPUs | mutual interference, slack/card |
| T4 | Strand-Rust-Coder 14B hybrid | 1070 Ti | control, expected loss |

Gates: **G1** Mellum ≥25 t/s · **G2** co-resident ≥500 MB slack/card · **G3** Qwen IQ2_S
≥8 t/s @32k · **G4** maintainer eyeballs IQ2_S quality vs IQ3_S.

## Phase 1 — libvitriol/rebis.py

- Mandatum packet JSON: `{objective, invariants[], file_slice{path,start,end,content},
  constraints[], output_contract}` — invariant block FIRST (stable prefix → prefix-cache
  hits), volatile slice LAST.
- Loop: Mellum draft → code-fence extract → compiler gate (`cargo check`/`brievc`) →
  Qwen verdict `{pass, delta[]}` → ≤3 iterations → human escalation.
- A/B harness: same task Qwen-solo vs Rebis; log TTFT, t/s, iterations-to-green.
- Server management extends sweep_controller pattern (spawn, /health poll, kill).

## Phase 2 — Anticipatio (shadow prefill)

Async fire-and-forget `/completion` (max_tokens=1, cache_prompt=true) to Qwen with the
same stable prefix whenever a Mandatum goes out. Measure next-turn TTFT delta.
Caveat: Mellum SWA(1024) may limit its own long-prefix reuse; Qwen side unaffected.

## Phase 3 — surface

OFFICINA `/rebis` command or opencode provider pair — decided on Phase 1 data.

## Risks

- IQ4_XS 6.8 GB on 8 GB card is knife-edge (~8.1–8.3 GB w/ q4_0 KV @16k) → ladder ready.
- Two page-locked DMA processes double locked RAM — watch `free -m`.
- Thinking variant `<think>` blocks burn tokens at Pascal t/s; if iterations-to-green
  suffers, try official Instruct MXFP4_MOE as drafter instead.
- Loop thrash mitigated by hard iteration cap + compiler objectivity.

## Progress log

- 2026-08-21 17:41 — P0.0 PASSED (arch + vocab + MXFP4 kernels all present). No fork
  changes needed. Mellum i1-IQ4_XS downloading.
- 2026-08-21 20:40 — **Phase 0 COMPLETE, all gates passed.** Full numbers in
  EXPERIMENT_LOG.md. Summary:
  - T0 baseline Qwen IQ3_S stream: prefill 264/262/239 t/s @1k/4k/16k, decode 20.4.
  - T1 Mellum i1-IQ4_XS pinned 1070 Ti: decode **69.8 t/s**, VRAM 6.68/8 GiB,
    prefill 559/513/442. G1 PASS.
  - T3 co-resident (+ Qwen UD-IQ2_S resident 3060 @32k): GPU0 9.83/12, GPU1 6.72/8 —
    G2 PASS. Qwen IQ2_S: prefill 428/438/417/394 (@32k), decode 19.6 — G3 PASS.
    Mellum unaffected: 70.2 t/s co-resident.
  - Concurrent load: zero contention penalty.
  - **Bonus finding:** IQ2_S-resident beats IQ3_S-streaming on prefill by 1.7× at
    equal decode — Pascal DMA crossing removed from Qwen's path entirely.
  - G4 (IQ2_S quality eyeball) pending maintainer.
  - T4 Strand control deferred (drafter gate passed by wide margin).
- Next: Phase 1 live loop test — rebis.py against ports 8287/8279 with a real task.
- 2026-08-21 21:05 — **Phase 1 smoke test PASSED** after two protocol fixes (empty-draft
  false-GREEN; verifier now reviews every draft). Full loop: Mellum draft → cargo gate →
  Qwen verdict → ACCEPTED iteration 1 with invariants satisfied. Details in
  EXPERIMENT_LOG.md. New files: `libvitriol/rebis.py` (selftest green),
  `libvitriol/prefill_probe.py`, `libvitriol/examples/rebis-example-task.json`.
- Open items: verifier prompt should demand invariant-by-invariant justification
  (Box::from_raw provenance slipped through as "good enough"); Anticipatio shadow-prefill
  not yet wired; A/B baseline mode untested against a real second task set.
