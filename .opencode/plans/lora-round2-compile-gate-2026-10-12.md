# LoRA Round 2: Compile-Gated Deep Eval (Owner-Approved Design)

**Date**: 2026-10-12
**Status**: DESIGN APPROVED — execution follows in this session
**Predecessor**: `.opencode/plans/lora-tryA-opus-reasoning-2026-10-12.md` (round 1: 400-tok prose A/B)

## Owner thesis under test

"If it creates the code faster, and a system like Praetor or the Rust
compiler can catch it, this is a win." Round 1 showed the adapter writes
more code, faster to the deliverable, with ~3/12 confident slips — exactly
the class a compiler arbitrates. Round 2 measures whether speed + machine
verification nets a win.

## Approved decisions

1. **Multi-round repair loop** (emulates live agentic loop): on gate
   failure, compiler/tool stderr is fed back as a user turn; up to
   K=3 repairs after the initial attempt (max 4 generations/problem).
2. **Clippy tier included**: green requires rustc compile + clean run
   (self-test asserts, exit 0) + `clippy-driver -D warnings` clean.
3. **Problem list** (6 hard single-file Rust, std-only, self-testing):
   Treiber lock-free stack, SPSC ring with correct orderings, generational
   handle arena, zero-copy nested TLV parser, Red-Black tree with
   invariant verification, single-threaded mini-executor (manual Waker).
4. **Verification = compiler only** this round; Praetor deferred as
   tie-breaker if results are ambiguous.
5. **Depth**: max_tokens 3500 (9× round 1), temperature 0. Prompt depth
   stays shallow — the deferred depth-eos bug (external draft ≥44k) is not
   in play; internal MTP handles long generations.
6. Arms: baseline = live unit untouched; adapter = same argv + `--lora`
   (fingerprint discipline as round 1; unit stopped for adapter arm,
   restored after).

## Gate definition (green)

| tier | command | pass condition |
|---|---|---|
| 1 compile | `rustc --edition 2021 -O -o bin main.rs` | exit 0 |
| 1 run | `./bin`, timeout 15 s | exit 0 (self-test asserts) |
| 2 lint | `clippy-driver --edition 2021 -D warnings -o discard main.rs` | exit 0 |

Failure of any tier = repair round with the failing stage's stderr (≤6 KB)
returned to the model. No repair budget for clippy-only dirt beyond K.

## Metrics per problem/arm

first-shot-green, green-within-K, rounds-to-green (0..3), generations,
completion tokens total, wall time total, mean predicted t/s, run-pass,
`unsafe` keyword count, LOC. Summary verdict: "time-to-green × quality"
per arm — the direct cost model for the owner's agentic-loop thesis.

## Toolchain

rustc 1.94.1 (e408947bf 2026-03-25), clippy-driver 0.1.94. Harness:
`/tmp/opencode/lora/gate/{problems.py,run_gate.py}` (transient, like
round 1; raw transcripts + per-attempt logs under
`/tmp/opencode/lora/gate/<arm>/<problem>/`).

## Non-goals

- No launcher/profile wiring yet (that follows an ADOPT decision).
- No ctx_dft priming patch (adapter arm runs with the known −17% penalty;
  the eval measures content per token, t/s recorded for the cost model).
- No context-depth testing (separate deferred workstream).
