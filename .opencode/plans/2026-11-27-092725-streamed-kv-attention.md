---
schema: "codex-work-plan/v1"
created_at: "2026-11-27T09:27:25+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "169aa16b72a992eae86f66ad0d63c69630353203"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-27-081932-kintsugi-224k-daily-driver.md"]
---

# streamed-kv-attention

## Purpose

Reduce PCIe stalls during 200K inference while preserving full attention.

## Approach and alternatives

Compare the existing reference staging path to chunked KV streaming, online softmax and overlap with verification.

- Keep the measured current profile unless paired experiments meet the acceptance criteria.

## Scope and documentation

engine/ggml/src/ggml-cuda/ggml-cuda.cu, fattn kernels, engine/common/speculative.cpp and kernel provenance.

## Verification

- Run the concrete acceptance and regression checks below with exact launch fingerprints and evidence outside the repository.

## Results and evidence

Planned. Record measured outcomes and verification evidence here during execution.

## Change history

- 2026-11-27T09:27:25+01:00 — Plan created before implementation.

## Baseline and hypothesis

The reference Kintsugi profile keeps 112,640 KV cells on GPU1 in a
229,376-token window. It already stages host KV and prefetches some work.
The measured 200K workload ranges from about 20 tok/s on fresh code to
39.6 tok/s on one revision. Those distinct workloads need separate controls.

Hypothesis: processing older KV in chunks can overlap PCIe copies with compute
and share each copy across speculative verification queries.

## Research and source comparison

Inventory and hot-file diff the reference CUDA staging, prefetch, attention
and speculative verification paths against VITRIOL first. Read SpecStream's
online-softmax and committed-history design, and determine which parts the
reference already implements. The paper's reported throughput is not a Z440
performance estimate.

- [SpecStream](https://arxiv.org/abs/2609.33184)

## Implementation stages

1. Measure transfer bytes, copy durations, stream waits, attention time and
   verification time at 32K, 117K and 200K using matched prompts.
2. Select the smallest missing mechanism demonstrated by the source diff.
   Prototype bounded double-buffered KV chunks with existing allocators and
   events. Avoid copying complete history before the first chunk can compute.
3. Accumulate attention with online softmax across all chunks and reuse staged
   chunks across the queries within a verification round.
4. Retain speculative candidate and rollback state on-device. Keep only
   committed history eligible for asynchronous offload.
5. Test cancellation and state restore during transfers, then compare against
   the unmodified reference and current resident path.

## Invariants

1. Disabling streaming reproduces the existing dispatch and cache behavior.
   Pin with golden backend-dispatch and lifecycle cases.
2. Every permitted attention position is processed exactly once with the same
   causal mask. Pin with dense attention oracles across chunk boundaries,
   quantized KV types and partial final chunks.
3. Rejected candidate state never enters committed host history. Pin with
   forced rejection, rollback and subsequent checkpoint restore.
4. Buffers remain owned until copy and compute consumers finish. Pin with
   cancellation/reuse races and CUDA memory/race checks on supported paths.

## Acceptance and falsification

First require kernel numerical tolerances appropriate to Q4 KV and a matched
quality evaluation. Compare code copying, early instructions, tool calls and
complete coding tasks. Then run paired latency tests at actual filled depths.
Provisional target: at least 15% lower median decode time at 200K, with no
material shallow regression and within the existing measured memory margin.
If transfer waits are small, or extra synchronization consumes the savings,
stop and document the negative result before broadening the kernel work.

## Dependencies and rollback

Requires telemetry and a current reference diff. Gate behind a backend option
and retain existing staged attention. Read the binding C++ guide before edits
and run applicable CUDA operator tests, numerical checks and Praetor gates.

## Evidence and delivery

Status is planned. Implementation and performance are unverified. Record per-stage commits, full argv, model/runtime identity and actual filled token counts. Use the repository verification gates, retain existing diagnostics and document unavailable checks. Save results before the final verification freeze. Public research code requires license review and attribution before incorporation. Live maintenance requires an idle or operator-authorized window.

## Related implementation plans

- [cache-continuity](2026-11-27-092725-cache-continuity.md)
- [adaptive-speculation](2026-11-27-092725-adaptive-speculation.md)
- [selective-historical-kv](2026-11-27-092725-selective-historical-kv.md)
- [inference-dashboard-telemetry](2026-11-27-092725-inference-dashboard-telemetry.md)
