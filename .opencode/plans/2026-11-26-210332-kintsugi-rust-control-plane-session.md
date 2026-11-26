---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T21:03:32+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "1c80aa2f87a2e1a14be61cd392140686d7233fb5"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-192713-kintsugi-200k-daily.md"]
---

# kintsugi-rust-control-plane-session

## Purpose

Emulate a complete local long-context coding session by having Kintsugi write a reviewable Rust control-plane prototype using VITRIOL source context.

## Approach and alternatives

Use the previously measured single-GPU reference MTP profile and a bounded user service. Feed about 202,000 tokens of actual VITRIOL source context, use a native read-only tool call, require a complete implementation and follow-up edit, and test session recovery from an isolated saved slot. Keep the crate separate from the live tree.

- A small pure router makes regressions independently testable. A full serving
  rewrite would mix model output quality with CUDA, transport and operations
  work. The prototype leaves those responsibilities with their existing owners.

## Scope and documentation

Use committed Rust and JavaScript source as read-only context and an explicit
fixture for the contract. The standalone scratch crate covers the Lapis
admission decision only. The dedicated port, slot directory, transcript,
tests and generated files live under `/tmp/vitriol-kintsugi-rust/`.

## Verification

- Record actual input/cache/decode counts, tool arguments and result, full generated source, independent Cargo tests, saved-slot restore, GPU headroom and production state. Reject a prototype that cannot compile or breaks key routing behavior. Compare restored prefix reuse with a cold slot; record unsupported API or timing properties explicitly.

## Invariants

1. The prototype implements a deterministic, pure Rust routing policy. A fitting idle local slot wins, an oversized or busy local slot is distinguished, and a protected request cannot escape to cloud. Independent Cargo tests pin each outcome.
2. The tool call reads only the named synthetic fixture. The transcript records the model's own structured arguments and result.
3. Cache-recovery evidence binds the saved context to a new server PID and reports actual reused tokens. Failing to restore a slot does not count as reuse.
4. The production model, profile, services and code remain at their entry state. The launcher uses a separate port, service and slot path. Repository changes are limited to the trial report and its summary.

## Results and evidence

2026-11-26T21:23+01:00. The reference engine ran the one-slot 229,376-token
GPU1 profile with MTP, lookup drafting, 112,640 GPU KV cells and int8 prefill.
The corpus used 32 committed files from `1c80aa2`. The last file was
trimmed to the corpus budget. The engine kept at least 1,015 MiB free on GPU1
and 11,103 MiB host available. The harness used one HTTP function tool
restricted to a synthetic fixture and did not forward any request to cloud.

| Turn | Prompt tokens | Cached tokens | Output tokens | Decode tok/s | Wall time |
| --- | ---: | ---: | ---: | ---: | ---: |
| Native tool call | 201,860 | 0 | 29 | 19.13 | 907.38 s |
| Rust implementation | 202,146 | 201,889 | 638 | 19.92 | 35.01 s |
| Test-driven revision | 203,288 | 202,784 | 688 | 39.64 | 21.56 s |
| First turn after restart | 204,040 | 203,976 | 38 | 19.87 | 4.26 s |

The native call was `read_fixture({"path":"fixtures/route-policy.txt"})`.
The first Rust draft compiled and passed six of seven independent cases.
It rejected a protected request before checking whether an idle local slot
fit. The follow-up included that failure and asked for the specific local
reason when a protected request cannot run locally. Kintsugi returned a
complete revised file and passed all seven cases. Draft acceptance was
481/627 on the implementation and 636/677 on the revision. The faster
revision is a single workload measurement.

The slot saved 203,976 tokens and 3,927,841,612 bytes in 2.12 s. A new
server process (432785 → 461841) restored the full slot in 1.97 s,
excluding process startup. The next turn evaluated only 64 new tokens
and correctly returned the session tag and `(Lane::Reject, Reason::LocalBusy)`.
The test server and driver exited, and the report retains the API results
while the large scratch snapshot was removed. The production system
services were inactive at entry and exit and the profile was untouched.

The standalone archive is
`/tmp/vitriol-kintsugi-rust/vitriol-route-prototype.tar.gz`. Its extracted
crate passed `cargo test --offline`; the source passed `cargo fmt --check`.
The experimental integration boundary is the existing caller that obtains
slot telemetry and needs a pure route decision. The crate has no network
or CUDA code. Its API and tests are independent of the live stack.

The private evidence directory `/tmp/vitriol-kintsugi-rust/evidence/`
contains full launch fingerprints, the corpus manifest, all API replies,
memory samples and independent Cargo logs. Verify changed scope against
`1c80aa2` with evidence under `/tmp/vitriol-kintsugi-rust/verification/`.

## Change history

- 2026-11-26T21:03:32+01:00 — Plan created before implementation.
- 2026-11-26T21:23+01:00 — Local tool, complete revised crate, independent tests and restart reuse observed. Retained the crate separately for future integration review.
