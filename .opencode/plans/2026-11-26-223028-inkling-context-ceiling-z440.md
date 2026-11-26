---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T22:30:28+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "cf973e8463427dc0d1340b4f4ee51572499b59f2"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-215926-inkling-officina-parser-qualification.md"]
---

# inkling-context-ceiling-z440

## Purpose

Measure the largest usable context window for stock b12774 and the same Inkling IQ4_NL model on two RTX 3060s.

## Approach and alternatives

Run one isolated process at a time. Start with Q4_0 target KV at 131,072 and increase the window until a ready-plus-generation probe fails its memory margin or functional check. Adjust layer placement only when logs show a specific imbalance. Measure filled recall separately with synthetic source and calculate warm reuse on the best practical candidate.

- Compare GPU-resident and host KV separately. Smaller microbatches trim
  compute reservations and can slow cold prefill. Host KV also changes
  the attention path and must be measured independently.

## Scope and documentation

Use the existing b12774 binary, official IQ4_NL weights and a 1:3 layer
split. The harness and artifacts live under `/tmp/vitriol-inkling-context/`.
No Officina model catalog or production engine configuration is changed.

## Verification

- Record full argv, exact advertised window, cold and warm prompt tokens, recall, generation, VRAM on both cards, host headroom, and engine cleanup. Compare against 32K F16 control and old 131K result; preserve production state. A load or shallow completion is capacity evidence and never counts as full-depth validation.

## Invariants

1. The test uses the same IQ4_NL weights and the upstream Inkling parser. All requests stay local and use an isolated slot with one server at a time.
2. Readiness, short generation, memory margin, and filled recall are distinct outcomes. Require at least 384 MiB free on each GPU after a short generation and preserve host memory margin. VRAM and host telemetry gate further requests.
3. The production units and model profile retain their entry state. Failures and timeouts stop only the test process.

## Results and evidence

### Ready-plus-generation boundary

The local GGUF declares a 1,048,576-token training context. On this host
the scan used one slot, Q4_0 K and V, GPU-resident weights, a 1:3 layer
split and a 27-token chat canary. Each pass required a correct answer and
384 MiB free on each card after generation. The lowest observed margin
is listed below. These measurements record setup capacity and short
generation, with filled-depth probes handled separately.

| KV placement and microbatch | Largest passing window | Smaller free GPU margin after short reply | Larger tests |
| --- | ---: | ---: | --- |
| GPU, 32 | 196,608 | 395 MiB | 212,992 and 229,376 below reserve; 262,144 failed initialization. |
| GPU, 16 | 212,992 | 475 MiB | 229,376 left 333 MiB before generation. |
| GPU, 8 | 229,376 | 385 MiB | 245,760 and 262,144 initialized below reserve. |
| GPU, 4 | 229,376 | 389 MiB | 245,760 left 303 MiB before generation. |
| Host, 4 | 401,408 | 399 MiB | 409,600 answered with 357 MiB free; 425,984 and 458,752 initialized below reserve. |

Host KV at 524,288 with ub8 reserved 4,608 MiB of global KV in system
RAM, then failed a 2,841.70 MiB GPU0 compute-buffer allocation. Moving
KV to RAM therefore leaves a GPU compute-reservation limit in this
runtime. The tested host configuration keeps the weights split across
both GPUs. These are measured brackets under the machine's observed
desktop load, with unspecified points between the tested sizes.

### Filled-depth measurements

The 401,408-token host-KV profile correctly recalled `BIRCH-871` with
4,164 filled tokens. Cold prefill took 87.49 s at 47.59 tok/s and decode
ran at 14.50 tok/s. Its warm follow-up sent GPU0 down to 291 MiB free
and was stopped before a reply under the test's memory reserve.

The 229,376-token GPU KV profile at ub8 recalled the same marker at
4,164 filled tokens. Cold prefill took 42.07 s at 98.97 tok/s and
decode ran at 23.52 tok/s. GPU1 reached 339 MiB free as the next
prefill began. The harness stopped that arm and fell back to the
212,992-token GPU KV profile at ub16 for the deeper probe.

| 212,992 GPU KV, ub16 | Filled prompt | Cached prompt | Exact answer | Decode | Wall time |
| --- | ---: | ---: | --- | ---: | ---: |
| 4K baseline | 4,164 | 0 | BIRCH-871 | 23.08 tok/s | 32.94 s |
| 64K source recall | 65,605 | 12 | BIRCH-871 | 12.35 tok/s | 716.90 s |
| Conversation follow-up | 65,635 | 65,601 | ASPEN-642 | 12.48 tok/s | 8.53 s |

The 64K source addition effectively required a cold prefill. The actual
follow-up reused 65,601 tokens and evaluated 34 new tokens in 1.10 s.
The 64K prefill held 423 MiB free on GPU1. Two consecutive samples
dipped to 371 MiB during the warm reply. The test finished before the
three-sample stop threshold. The observed 212,992 configuration thus
completed the deepest test with a narrow margin. The 196,608 ub16
point had 661 MiB free after its short response and is an alternative
when more reserve matters. Both maximum capacities remain unverified
near their actual fill limits.

All transient units exited with no remaining engine PIDs. Production
engine and autosave began and ended inactive. The devices returned to
their entry free-memory readings. See `/tmp/vitriol-inkling-context/`
for full per-arm fingerprints, scan JSONLs, response JSONs, memory
samples and `cleanup.log`. Run changed-scope verification against
`cf973e8`, with manifests in the scratch evidence directory.

## Change history

- 2026-11-26T22:30:28+01:00 — Plan created before implementation.


- 2026-11-26T22:32:40+01:00 — Q4_0 ub32 passed at 196,608, while 212,992 and 229,376 opened below the memory floor and 262,144 failed startup. The runtime logs show compute-buffer pressure during graph reservation. Try ub16 and ub8 to increase capacity while keeping the same weights and KV format. A wider window that fails a short generation, breaches the reserve or cannot progress through a deeper prompt does not qualify as practical. Bound the filled trial to a representative 64K–96K sample if the resulting window is much larger.

- 2026-11-26T22:33:50+01:00 — ub16 passed 212,992 and ub8 passed 229,376 with only 385 MiB free on GPU1 after a short reply. Probe ub4 for the all-GPU capacity boundary. Separately test stock host KV at a larger configured window. Host KV may expand capacity at the cost of prompt and decode performance, so report a shallow host pass independently from the practical GPU KV candidate.

- 2026-11-26T22:53+01:00 — Completed the capacity and 64K filled-depth comparison and verified cleanup.
