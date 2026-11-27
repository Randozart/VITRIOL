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

# cache-continuity

## Purpose

Retain reusable conversation prefixes across project switches and branches without repeatedly prefilling 200K tokens.

## Approach and alternatives

Extend existing slot persistence with bounded prefix and branch retention, including recurrent and drafter state.

- Keep the measured current profile unless paired experiments meet the acceptance criteria.

## Scope and documentation

scripts/lull_slot_persist.py, engine/tools/server/server-context.cpp, engine/common/speculative.cpp, Officina session handling and cache documentation.

## Verification

- Run the concrete acceptance and regression checks below with exact launch fingerprints and evidence outside the repository.

## Results and evidence

Planned. Record measured outcomes and verification evidence here during execution.

## Change history

- 2026-11-27T09:27:25+01:00 — Plan created before implementation.

## Baseline and hypothesis

The qualified reference build restored 203,976 tokens after a restart. The next
turn evaluated 64 new tokens in the earlier Rust session. Its cold read took
about 15 minutes. Production currently uses one slot and a five-minute autosave.
Whole-slot restart recovery does not establish reuse across project branches.

Hypothesis: retaining complete state at useful branch points reduces repeated
prefill enough to outweigh save, restore and storage costs on this workstation.

## Research and source comparison

Read SGLang HiCache's hybrid-state recovery and incremental branch backup before
designing a new format. Compare the reference implementation under
`~/kintsugi/engine-src/` with the in-tree cache and speculative state paths.
Inventory serialization of target KV, recurrent state, draft state and tokens.
Record missing components and licenses before adapting implementation code.

- [HiCache design](https://www.sglang.io/blog/hicache)
- [Incremental branch backup](https://github.com/sgl-project/sglang/pull/33639)

## Implementation stages

1. Replay two project histories that share prefixes, switch A → B → A, and
   branch from an earlier turn. Record exact-token prefix hits and actual
   recomputation before changing persistence.
2. Define a versioned cache identity containing model digest, tokenizer,
   template, cache format and state-affecting configuration. Audit the existing
   identity before adding fields. Retain compatible entries only.
3. Add bounded branch retention to the existing cache owner. Start with whole
   checkpoints, then compare incremental writes if full writes dominate.
   Commit metadata atomically and protect readers from partial writes.
4. Connect restore to session selection and retain useful branch points under
   a configurable disk/RAM budget. Verify cancellation, concurrent saves,
   crash recovery and eviction while a state is in use.
5. Expose cache origin and restored/new token counts through the telemetry plan.

## Invariants

1. With branch retention disabled, existing slot save/restore behavior matches
   the baseline. Pin with the existing lifecycle fixtures plus a golden trace.
2. A restored state matches the exact prefix and all required model state.
   Pin with divergent suffixes, recurrent/draft continuation and tool turns.
3. A crash or cancelled save preserves the last complete entry. Pin with
   interrupted writes and restart tests.
4. Concurrent readers cannot observe partial state or eviction of a pinned
   entry. Pin with explicit interleavings and a state-machine model if ownership
   crosses asynchronous queues.

## Acceptance and falsification

Use paired 32K and 200K A/B/B/A runs, including restart and an earlier-turn edit.
Report median and worst observed first-token latency, token hit rate, bytes
written, peak RAM/VRAM and CPU time. Provisional target: at least 90% prefix
reuse on the A → B → A case and at least 50% lower warm first-token latency,
with all state-consistency tests passing. Reject promotion if any incompatible
state is reused, draft recovery fails, or repeated switches cost more overall.

## Dependencies and rollback

Start with telemetry definitions. Persist behind an opt-in setting and keep
existing checkpoints readable. Roll back by disabling branch retention and
using the current complete-slot persistence path.

## Evidence and delivery

Status is planned. Implementation and performance are unverified. Record per-stage commits, full argv, model/runtime identity and actual filled token counts. Use the repository verification gates, retain existing diagnostics and document unavailable checks. Save results before the final verification freeze. Public research code requires license review and attribution before incorporation. Live maintenance requires an idle or operator-authorized window.

## Related implementation plans

- [streamed-kv-attention](2026-11-27-092725-streamed-kv-attention.md)
- [adaptive-speculation](2026-11-27-092725-adaptive-speculation.md)
- [selective-historical-kv](2026-11-27-092725-selective-historical-kv.md)
- [inference-dashboard-telemetry](2026-11-27-092725-inference-dashboard-telemetry.md)
