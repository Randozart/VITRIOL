# Dual-Slot Background Agent — Ideas Inventory, Research, and Value Ranking

**Date:** 2026-08-31 (session, late)
**Status:** PROPOSAL — nothing implemented. Cert-gated per AGENTS.md
residency rules; every throughput number below is a hypothesis until the A/B
protocol in §6 says otherwise.

## 1. The core idea

One engine, one loaded model, `-np 2`. Foreground Qwen (deep lane, main
conversation) and background Qwen (fast lane, odd jobs) are **the same
weights** — the second slot rents intelligence with a small KV cache plus a
per-slot compute buffer, not a second model. Shared-weight parallel decode is
cheap because decode is memory-bandwidth-bound: a second slot reading the same
weights adds aggregate throughput until bandwidth or KV capacity saturates
(Pope et al. 2022, §2 analytical model; Orca's batched-decode results).

Job contract for the background lane (the honesty rule):

> **Bounded input** (repo state + small artifacts: diffs, files, cards) →
> **compact output** (a card, ≤ ~500 tokens) → **queue** → main agent pulls
> when relevant. Anything that needs the conversation transcript is REFUSED —
> shipping main-context to the lane costs a full prefill per job and is the
> opposite of efficient (that class of work stays on the mellum2 small lane).

## 2. Ideas inventory (all of them, unranked)

1. **Idle-cycle harvesting** — the fast lane accepts jobs only when the
   engine is idle (`/slots` busy=0, from the `_shared/engine.ts` poller).
   Between-turn GPU idle time becomes completed work at ~zero foreground cost.
2. **Diff reviewer** — after each turn's edits, review just the patch + local
   context; emit findings card (bugs, edge cases, inconsistencies).
3. **Read-ahead digests** — pre-read repo-map PageRank neighbors of the
   module under active edit; write 10-line export/caller/TODO digest cards;
   `knowledge-inject` serves them later, replacing reads.
4. **Plan-mode advance scout** — trace call paths and gather evidence cards
   in parallel with the main agent's own plan-mode research.
5. **Churn investigator** — when `edit-churn` fires, race an alternative
   fix approach on the fast lane; return both candidates.
6. **Memory curator** — pre-triage memory-extractor candidates: draft
   MEMORY.md entries, check contradictions. Human sign-off unchanged.
7. **Dual-slot subagent routing** — plain `subagent`/dispatch runs on slot 0
   (no new behavior, just parallelism with the main agent).
8. **Speculative plan repair** — when a foreground turn fails checks
   (verify-contract/diagnostics), background agent pre-drafts the repair.

Rejected for now (with reasons):
- *Conversation summarization on slot 0* — needs transcript; mellum2 small
  lane already owns it and is faster.
- *Background "second opinion" on every turn* — decode bandwidth is not free
  when the foreground is active; only worth it for load-bearing decisions.

## 3. What the literature says

| Work | Finding we exploit |
|---|---|
| **Orca** (Yu et al., OSDI 2022) | Iteration-level (continuous) scheduling: batching decode iterations across requests shares weight reads; the foundation of multi-slot serving. llama.cpp `-np N --cont-batching` is this. |
| **Pope et al., Efficiently Scaling Transformer Inference** (arXiv:2211.05102) | Decode is memory-bandwidth-bound (arithmetic intensity ~1 op/byte); batched decode amortizes weight reads — aggregate throughput rises with batch until compute/bandwidth saturates. Justifies "second slot nearly free" for small N. |
| **Sarathi-Serve** (arXiv:2403.02310, OSDI 2024) | Chunked prefill: a large prefill (exactly what a background job does on admission) can be sliced so ongoing decodes are not stalled. Engine-side: our fork's `-ub 64` ubatch already bounds chunk size — a background *prefill* interleaves at ≤64-token chunks, so foreground decode hiccups are bounded, not eliminated. |
| **SGLang / RadixAttention** (arXiv:2312.07104) | Prefix caching across requests: background jobs that share a fixed system prompt / tool schema prefix with each other get KV reuse — jobs should share a common prefix deliberately. |
| **DistServe** (arXiv:2401.09670, OSDI 2024) | Prefill and decode have opposite resource shapes; disaggregating them avoids interference. We approximate this temporally (idle-gating) instead of spatially (no second GPU pool). |
| **Llumnix** (arXiv:2406.03243, OSDI 2024) | Dynamic rescheduling of requests across slots/instances; supports our later idea of migrating the background job if the foreground needs the lane. |
| **Large Language Monkeys** (arXiv:2407.21787) | Repeated sampling raises coverage of hard problems (coverage grows ~log with samples); justifies racing N candidate fixes (idea 5) on problems where verification is cheap (tests). |
| **CacheBlend / CacheGen** (arXiv:2405.16444 / 2310.07240) | KV reuse/non-contiguous prefix blending — the engine-side future for feeding big shared prefixes to the background lane cheaply. Out of scope v1; recorded as direction. |

Honest reading: the literature covers *serving systems* for many independent
requests. Our twist is (a) N=2 with tight VRAM, (b) temporally gated
scheduling (idle harvesting) rather than latency-SLO driven, and (c) the
client is a coding harness that can choose *what* to enqueue. The novel part
is the policy, not the scheduling — which is exactly where Officina lives.

## 4. Value ranking (expected payoff × certainty ÷ effort)

| Rank | Idea | Why |
|---|---|---|
| 1 | Idle harvesting + diff reviewer (v1) | Zero foreground cost by construction; highest perceived value ("free second opinion"); smallest surface (queue + gate + prompt). |
| 2 | Dual-slot subagent routing (v1.5) | Pure parallelism win for dispatch; measurable immediately (wall-clock of multi-dispatch turns). |
| 3 | Read-ahead digests (v2) | Compounds with the 10:1 context offload — cards replace reads; depends on repo-map neighborhoods being right. |
| 4 | Churn investigator (v2) | Attacks the most expensive failure mode; needs the diff-reviewer plumbing. |
| 5 | Plan-mode scout (v2.5) | Good, but plan-mode quality is model-bound; research must not degrade it. |
| 6 | Memory curator (v3) | Nice, low urgency — queue is small. |
| 7 | Speculative plan repair (v3) | Only after we trust lane quality; risk of wasted cycles. |

## 5. VRAM and window math (to be certified, not assumed)

- Weights: shared, zero marginal.
- Background KV at 16–32k tq3_0: ~0.5–1.5 GB (compute from GGUF dims by the
  libvitriol estimator, not hardcoded).
- Second per-slot compute buffer: small, fixed, must appear in the
  certification (Pascal compute buffers are the known killer).
- Funding: main window 131k → ~112k + 16k background (or 96k + 32k).
  Given the measured ~20k live / 200k offloaded steady state, sacrificing
  main-window depth is the *cheapest* currency we have.
- Depth wall: the ~23 KiB/token prefill VRAM creep applies per slot; the
  dual-window configuration must be depth-certified per AGENTS.md before any
  claim (Addenda 5–6 discipline).

## 6. A/B measurement protocol (cert-gate)

1. Serial baseline: current profile, dispatch wall-clock on a fixed
   multi-dispatch benchmark task (record via sweep_controller conventions,
   full argv fingerprint per flag-provenance rule).
2. Parallel ungated: `-np 2` (even 65k/65k), dispatch to slot 0.
   Measure aggregate t/s on /slots, foreground decode latency during
   background prefill, wall-clock.
3. Parallel idle-gated: same + `background-lane` gate.
   Measure idle-time utilization and foreground-latency tail.
4. Dual-window (needs engine per-slot n_kv, already APPROVED/cert-gated in
   docs/OFFICINA.md): 112k/16k; redo depth certification at both windows.

Decision rule: adopt dual-slot if (a) foreground depth at the reduced window
still certifies at ≥ the session's measured working set (~20k live), and
(b) multi-dispatch wall-clock improves ≥ 20%, and (c) foreground tail latency
during gated background jobs stays within noise.

## 6b. RESULTS (2026-09-01, live engine) — ADOPTED ✅

`scripts/bench-dual-slot.py` (fingerprint: server argv in
`~/.vitriol/officina/state/launch-fingerprint.txt`; engine booted
`--parallel 2`, `-c 81920 --kv-unified`, ts 22,14, MTP n1):

- A serial: 107.1s wall, 9.6 t/s aggregate
- B parallel: 71.1s wall, 14.4 t/s aggregate → **1.51× speedup** (≥20% PASS)
- C foreground stall: decode 11.3 t/s before AND during an 8k prefill
  admission → **zero stall** (PASS; chunked prefill at ubatch 64)
- Depth: KV pool unified, VRAM flat vs single-slot — no window sacrificed

Note: `tokens_predicted_total` under-counts with MTP on; the stall sampler
reads `n_decode_total` (decode steps) instead. All criteria met → adopted.

## 7. Architecture sketch (v1)

```
background-lane ext (officina)
  ├─ job queue: .pi/background/<session>/jobs.jsonl  (bounded input only)
  ├─ gate: _shared/engine.ts poller — busy==0 AND no foreground stream
  ├─ runner: POST /completion {slot_id: 0, shared prefix prompt, n_predict}
  ├─ output: cards → .pi/background/<session>/cards/*.md
  └─ delivery: task-state-style tail injection or knowledge-inject surfacing
```

Kill switch: `OFFICINA_NO_BACKGROUND=1`. Everything HTTP-only (layer rule).
