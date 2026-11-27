---
schema: "codex-work-plan/v1"
created_at: "2026-11-27T12:53:06+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "7a6852b8ee0fe49609674c40c892acad6dd6c5f8"
supersedes: []
superseded_by: []
related: []
---

# model-assisted-rust-caduceus-port

## Purpose

Use real Officina coding turns to extend the reusable VITRIOL Rust core, retaining reviewed local-model output and measured inference behaviour

## Approach and alternatives

Have Kintsugi implement a pure Caduceus policy module in an isolated scratch checkout; test against independent JavaScript golden vectors, integrate passing code into vitriol-core and retain measured turn receipts

- No alternative selected yet; record any material tradeoff before implementation.

## Scope and documentation

Identify affected files, contracts and documentation before implementation.

## Verification

- Cargo tests, JavaScript parity matrix, formatting/clippy, scoped Praetor, source-bound repository gates, local-only Officina receipts, unchanged existing API and saved-slot recovery

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-27T12:53:06+01:00 — Plan created before implementation.

## Invariants

1. Existing Rust-core APIs and live JavaScript routing behaviour remain unchanged. Existing Cargo tests and unchanged router-lib tests pin parity; the new pure module is opt-in for future callers.
2. The new Rust policy matches the supported typed JavaScript domain, including ordered deduplication, ECMAScript trimming, future-only parks, the phantom lane and forwarding-before-retry. Independent JS-generated fixtures pin the properties, with explicit numeric/nonfinite boundary tests.
3. Model edits are confined to a scratch checkout. Read-only-root bwrap and only file tools enforce this; tests/reference/manifests are hash-checked before integration. Raw model output and evaluator changes are attributed separately.
4. All inference uses the selected local reference engine. A loopback receipt proxy records request hashes, client first meaningful delta, wall duration and final usage. No cloud credentials or production catalogs enter the scratch environment.
5. Production service/profile are unchanged. Save the initially idle slot to a uniquely named checkpoint before coding, then restore only if the final idle task still belongs to this trial. If another request intervenes, preserve the new owner and report recovery as skipped. Endpoint/engine identity is recorded.
6. Generated text is allowed to vary. Correctness is decided by API contracts and independent behavioural tests, never byte-identical LLM answers.

## Scope and tradeoffs

Port candidate ordering, retry policy, capped park time, parked-state pruning and give-up reporting into `vitriol-core/src/caduceus.rs`. Reuse the established crate instead of creating another parallel router library. Integrate only after review and parity tests; no live router switchover or Officina frontend change. Two incremental coding turns exercise real file edits and an extension; bounded repair turns retain first-pass failures. Use actual source context and report filled counts as observed; no synthetic 200K claim. Larger-context qualification and Inkling comparison remain separate work requiring GPU capacity.

Reference source is the committed `officina/scripts/caduceus-router-lib.mjs`, compared with the previous scratch Rust prototype before running the trial. Existing prototype ASCII filtering does not establish Unicode parity. Core Rust API contracts will explicitly model string inputs and numeric timestamps; dynamic JS coercion is handled only where the typed interface represents it.

Evidence and scratch live under `/tmp/vitriol-rust-port-20261127/`. Required final gates include the whole core Cargo suite, formatting/clippy, deterministic JS fixture regeneration, router-lib tests, scoped Praetor and repository source-bound review/secrets/docs/build/tests. Freeze code and fixtures before final validation. Retain all nonpassing findings and avoid repairing unrelated baseline debt. Hypothesis is falsified if useful model-written code cannot pass independent parity without evaluator implementation, or if it changes existing routing behaviour.

### Stage 1 — 2026-11-27 13:05 +01:00 Europe/Amsterdam

Kintsugi used the installed Rust Officina TUI through a local-only provider and
real file tools. Sandbox setup first required exposing the existing Node binary
and using the proven private HOME catalog location; those attempts made no model
request. The first code turn took 80.04 s and wrote the module plus its export.
Independent evaluation found a test-key type mismatch, an incorrect self-test
expectation and missing U+2028/U+2029 trimming. The first repair (45.06 s) retained
the compiler error and introduced incorrect U+0085 trimming. The second repair
(33.52 s) used exact compiler diagnostics and observed Node results and passed.
Raw sources and each failure are retained; evaluator changes were mechanical
Rust formatting and adding the reviewed module export to the canonical crate.

All 33 scratch Cargo tests and clippy passed after repair, including independent
96-order-case Unicode/boundary vectors and exhaustive 0..599 retry statuses with
forwarding/timeout combinations. The starting-file audit confirms only src/lib.rs
changed among existing files; the model added src/caduceus.rs. Tests, fixtures,
reference and manifests were read-only. The original warm slot was saved under a
unique filename before any trial generation. The engine stayed PID 816552.
Stage 2 continues the same model coding session with new park-management code.
