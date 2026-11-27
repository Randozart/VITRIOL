---
schema: "codex-work-plan/v1"
created_at: "2026-11-27T12:53:06+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "7a6852b8ee0fe49609674c40c892acad6dd6c5f8"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-214613-officina-rust-port-model-comparison.md", ".opencode/plans/2026-11-27-092725-inference-dashboard-telemetry.md"]
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

- 2026-11-27 13:08 +01:00 Europe/Amsterdam — Stage-2 source review found an additional JavaScript boundary worth pinning before acceptance: with now=-1 and a future park timestamp of zero, giveUpMessage selects zero but its truthiness check reports no parked lane. Add this oracle-generated vector and preserve the quirk for compatibility; a future joint JS/Rust behavior fix is separate. Existing tests are unchanged. The first extension also failed the predeclared NaN-cap case and clippy's collapsible-if gate; feedback will distinguish those findings from the newly added boundary vector.

- 2026-11-27T13:13:53+01:00 — The two model-authored components are integrated into the existing Rust core after all 36 Cargo tests, formatting and clippy pass. The unchanged JavaScript policy suite passes under its correct Vitest runner; an initial node --test invocation was a runner mismatch, retained separately. Oracle regeneration is byte-stable, scoped Praetor has no blocking findings, and all trial processes stopped. The original idle slot restored 1512 tokens / 191166028 bytes in 425.132 ms on the same PID. Final source-bound repository gates follow the implementation commit.

## Final coding-trial results — 2026-11-27 13:14 +01:00 Europe/Amsterdam

The model wrote all five policy functions and the new module's own tests. The
evaluator supplied API specifications, independent tests, reference vectors,
compiler/Node/clippy feedback, mechanical formatting and repository integration.
No evaluator implementation patch was needed. Existing files changed only by the
module export; independent tests, reference and manifests remained unmodified by
the model. The oracle gained the documented zero-timestamp boundary during source
review, before accepting the extension. Stage-1 code is commit `7335b31`.

| Coding turn | Active wall s | Model requests | Generated tokens | Median decode tok/s | Maximum filled tokens | Outcome |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| Candidate/retry implementation | 80.04 | 5 | 1,983 | 36.77 | 10,994 | Compile and Unicode parity failures |
| First repair | 45.06 | 3 | 1,263 | 37.11 | 14,017 | Compile failure retained; U+0085 regression |
| Second repair | 33.52 | 2 | 1,231 | 37.81 | 15,559 | Full suite and clippy passed |
| Park-management extension | 90.44 | 5 | 2,747 | 34.92 | 21,486 | NaN-cap parity and clippy failures |
| Extension repair | 53.34 | 3 | 1,829 | 35.08 | 24,522 | All functional tests passed; clippy NaN idiom rejected |
| Final cleanup | 7.68 | 2 | 167 | 32.07 | 24,840 | Full suite and clippy passed |

The 20 requests all returned HTTP 200 through the direct local timing proxy.
Final SSE responses carry authoritative engine timings and usage. Across these
requests, median decode was 35.02 tok/s; median proxy-observed first meaningful
text/tool delta was 1.875 s, with a 14.751 s maximum on a source-reading follow-up.
The proxy uses monotonic timing around the native request and forwards events
immediately. This is observed proxy latency rather than native server TTFT.
Active turn time totals 310.08 s and excludes evaluator tests and feedback pauses.
Generated text was not required to be byte-identical between calls.

The original 1,512-token slot was saved as 191,166,028 bytes in 135.805 ms before
trial generation. The final idle task matched the trial's final receipt, so the
baseline slot restored in 425.132 ms; GET /slots then reported 1,512 filled tokens
again. The engine remained PID 816552 throughout and the production profile was
unchanged. The uniquely named baseline snapshot is retained for recovery. Trial
Officina and the timing proxy exited; no secondary engine or cloud inference ran.
The prototype is available through the Rust library, while the live router's JS
implementation and existing RPC contract remain unchanged.

Independent verification covers 96 candidate-order vectors, 600 statuses with
all forwarding/timeout combinations plus missing status, 35 cap vectors, three
pruning vectors, nine reporting vectors and native NaN/infinity boundaries.
All 36 Cargo tests, formatting and clippy pass. The original JS suite passes
30 tests under Vitest. Golden regeneration is byte-stable. Scoped Praetor on the
core source has zero blocking findings against a zero-blocking baseline; this
is a source scan, not a proof of every path. Repository-wide checks and unrelated
prior lint/security debt are outside this changed-core stage's claim.

Evidence under `/tmp/vitriol-rust-port-20261127/evidence/` includes the full
baseline engine argv and launch fingerprint, raw per-request SSE receipts,
actual slot/task timing lines, PTY/session traces, each emitted source version,
file audits, first-pass and repair diagnostics, guarded recovery, result hashes,
`trial-summary.json` and `result.json`. Final source-bound gates use base
`7a6852b` and live under `verification/`. This is one real coding workflow at
24,840 filled tokens, not a new 200K test or a model-wide quality comparison.
