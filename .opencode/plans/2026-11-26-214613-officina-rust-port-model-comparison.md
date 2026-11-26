---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T21:46:13+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "78dd695dbf193605511a4c851fa9da1d26372583"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-210332-kintsugi-rust-control-plane-session.md"]
---

# officina-rust-port-model-comparison

## Purpose

Run the canonical Officina Rust TUI with local Kintsugi and Inkling 2 Mini on a matched incremental Rust port of the Caduceus lane-order and retry policy.

## Approach and alternatives

Use two identical scratch crates and one task from real JavaScript source, direct local model catalogs and per-arm Officina settings. Tool reads and file mutations happen in the actual terminal client. Independently run Cargo parity tests, then provide bounded feedback if needed. Record model-visible tokens and wall time separately from GPU allocation.

- Use a shared workload that fits both installed profiles. Retain the previous
  200K Kintsugi result as a separate long-context experiment. If a backend
  emits tool syntax as text, preserve the failure and allow an explicitly
  assisted source-in-prompt arm to measure code generation.

## Scope and documentation

The only ported code is in the scratch crate. It extends the previous
Rust prototype with Caduceus `candidateOrder` and `shouldRetry`. The
installed Rust Officina TUI launches its original Pi RPC runtime through
a local shim that exposes file tools and uses a scratch model catalog.
Use separate workspaces per model. The profiles, emitted source, PTY
captures, API timings and tests live in `/tmp/vitriol-officina-rust-compare/`.

## Verification

- Check actual Officina session traces, ordered lane and retry regression tests, test-after-stream guard, local-only endpoint receipts, and cleanup. For a fair comparison both models use the same small task well within 32K. Report the separate prior 200K Kintsugi exercise as capacity evidence, with no assumed Inkling parity at that context.

## Invariants

1. Both arms start from the same unmodified Rust prototype and receive the same task, source and independent tests. The ordered-candidate and retry results must match the JavaScript policy.
2. The installed Rust TUI sends the prompts through its Pi RPC bridge. Per-arm scratch settings select only the intended loopback backend. No production settings or source files are changed.
3. Record first-pass behavior, test feedback or inability to use tools separately. A failed first turn can still provide evidence of compatibility, but cannot count as a completed port.

## Results and evidence

2026-11-26T21:53+01:00. The installed Officina binary drove the real Pi
RPC runtime against each direct loopback model. The harness exposed
read, edit, write, grep, find and ls in isolated scratch directories.
It selected the local provider from a private model catalog, while
independent Cargo tests ran outside the agent. Both arms began with the
same Rust source, JavaScript source and manifest. The final comparison
confirmed that neither changed the manifest or either test file.

| Arm | Configured context | First result | Follow-up | Offline tests |
| --- | ---: | --- | --- | --- |
| Kintsugi reference MTP, GPU1 | 229,376 | 46.00 s, read/read/edit/grep. Rust E0277 on two borrowed string comparisons. | 16.37 s, edit/read/edit in the reopened session after receiving the compiler diagnostic. | 11/11 after repair and from an unpacked archive. |
| Inkling 2 Mini IQ4_NL, VITRIOL engine, GPUs 0 and 1 | 32,768 | 19.22 s, emitted two intended reads as plain text. No tool executed and the project was unchanged. | 63.91 s assisted turn. Given matching JavaScript excerpts inline, it returned a complete code fence. | 11/11 after the evaluator appended the emitted code unchanged to a scratch copy. |

First-pass Kintsugi generated 1,130 tokens over four model requests,
with individual decode rates of 35.42–42.76 tok/s and a session usage
total of 7,578 tokens on the last request. The repair generated 474
tokens. The harness initially closed the TUI before the diagnostic
could be sent, so the repair reopened that same Pi session and used
the still-running Kintsugi endpoint. Reported per-turn times exclude
terminal relaunch and operator feedback time.

The current Inkling endpoint decoded the native attempt at 18.12
tok/s, and the assisted 818-token result at 17.83 tok/s. The first
attempt returned text beginning with `read{"name":"read","args":...` and
literal message delimiters. The installed stack did not expose those as
structured tool calls. This observed compatibility failure does not
establish that other Inkling servers or tool adapters behave the same.
Sampled GPU0 free VRAM was 533 MiB and GPU1 free VRAM was 1,321 MiB
while Inkling was resident. The compared task did not approach 32K.

JavaScript parity probes confirmed the Rust fixtures against the source,
including retry statuses across 100–599, equal-to-now park expiry, and
no retry after forwarding. Kintsugi passed `cargo fmt --check`. Model
responses, stage timings and build logs are under
`/tmp/vitriol-officina-rust-compare/evidence/`. Deliverables are the
Kintsugi crate archive `vitriol-rust-caduceus-kintsugi.tar.gz` and the
unaltered Inkling appendix `inkling-assisted.rs` in the same scratch
parent directory.

Both isolated engines and the terminal drivers were stopped. Production
engine and autosave began and ended inactive. No live model configuration
or integration code was changed. Check the report with changed-scope
verification against `78dd695`.

## Change history

- 2026-11-26T21:46:13+01:00 — Plan created before implementation.
- 2026-11-26T21:53+01:00 — Finished both Officina arms and the independent test comparison.
