# eval-gate: compile-gated A/B harness for model comparisons

Measures whether arm B earns its speed/quality keep against arm A on
verifiable Rust codegen: rustc compile + clean run (self-test asserts) +
`clippy-driver -D warnings`, with a multi-round stderr-feedback repair
loop that emulates a live agentic loop.

## Usage

```
python3 run_gate.py --port 8279 --arm base    --out results6k
python3 run_gate.py --port 8279 --arm adapter --out results6k
```

- Problems: `problems.py` (6 std-only single-file tasks with main()
  self-tests). Each problem gets up to `--rounds 3` repairs after the
  initial attempt; the failing stage's stderr (<=6 KB) is fed back as a
  user turn.
- Green = compile_ok + run_ok + clippy_ok. Per-attempt transcripts,
  compile logs and result JSONs land under `<out>/<arm>/<problem>/`.
- Tune `--max-tokens` ABOVE the largest problem's code size (a 3.5k
  budget truncated the rbtree into guaranteed failures; 6k is the
  working floor).

## Known limits (measured 2026-10-12, see
`.opencode/plans/lora-round2-results-2026-10-12.md`)

- temp-0 is NOT deterministic under MTP/batching: single-sample A/Bs
  cannot separate small deltas - run repeats.
- Whole-file-regeneration repair stalls on borrow-checker failures at
  K=3; surgical-edit agents need more rounds.
- Probe the gate first with known-good/known-dirty files before burning
  model tokens.
