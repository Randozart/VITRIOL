# LoRA Round 2 Results: Compile-Gated Deep Eval

**Date**: 2026-10-12 (session ~12:00–13:00 CEST)
**Status**: COMPLETE — thesis verdict: **not a win at K=3**
**Design**: `.opencode/plans/lora-round2-compile-gate-2026-10-12.md` (owner-approved)
**Predecessor**: `lora-tryA-opus-reasoning-2026-10-12.md` (round 1 prose A/B)

## Question

Owner thesis: "if it creates the code faster, and a system like Praetor or
the Rust compiler can catch it, this is a win." Round 1 showed the Opus-
distill adapter writes denser, code-first answers with ~3/12 confident
slips — machine-verifiable class. Round 2 put both arms through a
multi-round repair loop (live agentic-loop emulation) against a
rustc + run + clippy gate.

## Method

- 6 hard single-file Rust problems (std-only, self-testing main):
  Treiber lock-free stack, SPSC ring (correct orderings), generational
  handle arena, zero-copy nested TLV parser, Red-Black tree with
  invariant verification, single-threaded mini-executor (manual Waker).
- Repair loop: up to 3 fed-back failures per problem (max 4 generations);
  failing stage's stderr (≤6 KB) returned as user turn. Green = compile +
  clean run + `clippy-driver -D warnings`, strictest tier included.
- Sampling: temperature 0, max_tokens 6000 (3.5k budget invalidated by
  guaranteed truncation on rbtree — see round-2a note).
- Harness: `scripts/eval-gate/{problems.py,run_gate.py}`; raw transcripts
  `/tmp/opencode/lora/gate/results6k/{base,adapter}/<problem>/attempt*.rs`.

## Provenance

- Base arm = live unit untouched: `VITRIOL-FINGERPRINT: unit MainPID`
  (qwen38-buffedmod-262k blessed set, ts 27,21, ctx 262144, MTP n=3,
  q4_0 KV fa, cram=12288, ckpts=4, `--reasoning off`, port 8279).
- Adapter arm = manual launch, blessed argv + `--lora
  ~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf` (id=0 scale=1.0,
  load ~385 s): `VITRIOL-FINGERPRINT: manual-eval-launch r2-adapter-arm
  argv=unit-blessed-argv+--lora(...) ts=27,21 ctx=262144 spec=mtp:3
  kv=q4_0/fa ckpts=4 cram=12288`.
- Toolchain: rustc 1.94.1, clippy-driver 0.1.94. Gate sanity-tested with
  known-good/known-dirty probes before burning tokens.
- **Contamination note**: an unrelated foreign llama-server (Qwen2-VL-2B,
  `-ngl 0`, `-t 6`, port 8765, `/scratch/builds/build-ku-cu12`) ran
  CPU-resident during the adapter arm only. 8 cores total → host-thread
  oversubscription possible; decode `predicted_per_second` is measured
  server-side and matches round-1's clean −17% delta, so the decode
  comparison stands; wall-clock for the adapter arm is partially
  confounded (direction unaffected — see below).
- Unit restored active after (one SIGTERM-during-slot-save race with the
  manual process shutdown; systemd auto-recovered, health 200 verified).

## Results (max_tokens 6000, K=3)

| problem | base green/rounds/tok/wall | adapter green/rounds/tok/wall |
|---|---|---|
| treiber-stack | **Y**/3/5392/151 s | N/—/6972/238 s |
| spsc-ring | N/—/5119/148 s | **Y**/2/4381/150 s |
| handle-arena | N/—/7726/216 s | N/—/7689/262 s |
| tlv-parser | N/—/14144/439 s | N/—/**22719**/751 s |
| rbtree | N/—/17112/492 s | N/—/13936/472 s |
| mini-executor | N/—/9149/273 s | N/—/7794/282 s |
| **totals** | **1/6**, 58.6k tok, 1718 s | **1/6**, 63.5k tok, 2155 s |
| mean decode t/s | 36.6 | 30.9 (−16%) |
| cost per green | 1718 s | 2155 s |

## Findings

1. **Green rate: tie, 1/6 vs 1/6** — and the greens are *different
   problems* (base solved treiber, adapter solved spsc). With temp-0
   nondeterminism under MTP/batching (confirmed: base tlv was GREEN in
   the discarded 3.5k-budget run, FAIL in the 6k run), problem-level
   variance dominates arm-level signal at this sample size.
2. **The adapter is not faster end-to-end**: −16% decode (reproduces
   round 1's −17% — the unprimed `ctx_dft` + adapter matmul tax), +8%
   tokens (tlv thrash: 22.7k tokens across 4 attempts without
   convergence), +25% wall.
3. **The compiler catches the errors but neither arm converts feedback
   into green within K=3** at this difficulty. Failure stages cluster on
   borrow-checker-across-threads (Send/'static fights, Arc misuse,
   Pin/RawWaker borrows). These are exactly "agentic-loop solvable" —
   with more rounds and an agent that edits *surgically* rather than
   regenerating whole files — but K=3 whole-file regeneration doesn't
   get either model there.
4. **Adapter failure texture** (from round 1 + this round): fluent,
   canonical-looking code with occasional confident slips (round 1:
   `x/2 → x>>1` claim, pos+n overflow; here: 22.7k-token tlv thrash).
   Base failure texture: conservative, runs out of ideas/length. Neither
   dominates the other on quality; both are far from these 6 problems
   one-shot.
5. Round-2a (3.5k budget) retained at
   `/tmp/opencode/lora/gate/results/` — rbtree truncation there is a
   budget artifact, not signal.

## Thesis verdict

"Faster + compiler-catchable" **fails on its own terms at K=3**: the
adapter is slower per token, spends more tokens, and earns zero
additional greens from the repair loop. The remaining pro is purely
stylistic (round-1 density/structure, which the owner may or may not
prefer for design prose). The known −16% speed tax has a recovery path
(`ctx_dft` priming patch) that was intentionally not applied this round —
it would not change the green-rate finding.

## Recommendation

**Shelve** (default): keep `~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf`
+ converter fix (inner `cd55f1f3a`, upstream-grade regardless); live
config untouched; revisit only if a future base model or adapter
demonstrates a real green-rate or quality delta. **Drop** is reasonable
if round-1 prose style didn't win the owner over — the file is 467 MB
and nothing else depends on it. **Adopt** is not supported by evidence
from either round.

## Artifacts

- `scripts/eval-gate/{problems.py,run_gate.py}` — reusable compile-gate
  harness (rustc + run + clippy tiers, repair loop)
- `/tmp/opencode/lora/gate/results6k/{base,adapter}/` — per-attempt
  transcripts, compile logs, result JSONs
- `/tmp/opencode/lora/server-adapter-r2.log` — adapter-arm server log
- Green programs: `base/treiber-stack/attempt3.rs`,
  `adapter/spsc-ring/attempt2.rs`
