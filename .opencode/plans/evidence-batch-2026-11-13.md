# Evidence batch — E1 ubatch A/B + synth-GGUF loadability

- Date: 2026-11-13
- Status: implemented 2026-11-13.
  - Stage 1: depth_cert repaired (engine path + --spec-n + fingerprint).
  - Stage 2: 6 arms run. **ub128 wins: 42K +5.1% (all paired rounds),
    13K +0.9%, shallow noise. Verdict: keep 128; no live change.**
  - Stage 3: synth fix — the loader mismatch was `nextn_predict_layers=1`
    inherited from the MTP source header (the llama arch then expected an
    extra 9-tensor MTP layer). Dropped nextn/expert keys; llama-bench now
    loads the synth (pp ~15-16k, tg ~317 t/s).
  - Stage 4: adoption rule evaluated → ub64 did NOT win → live config
    untouched (already 128). Rollback profile irrelevant.
  - Stage 5: E1 table + verdict in docs/engine-settings.md; drift table
    entry marked RESOLVED.
- Decisions: run E1 now · depth points 13.1K **and** 42K · adopt in the
  live config if ub64 wins.
- Operator weighting (2026-11-13): "I work at depth far more than
  shallow, so I prefer performance gains at depth." → **shallow arms are
  informational only; adoption is decided on the DEPTH arms.** Adopt ub64
  if it wins ≥3% at 13.1K OR at 42K and is not >2% slower at the other
  depth point.

## Stage 1 — harness repair (`scripts/depth_cert.py`)

- `SERVER` was a stale `llama.cpp/build/bin/llama-server` path (the fork
  lives at `engine/`). → `engine/build/bin/vitriol-server`.
- `--spec` hardcoded MTP n=1; the live driver is n=2 → add `--spec-n`
  (passes `--spec-type mtp --spec-draft-n-max N`).
- Smoke gate: `--target 512 --ub 64 --kv q4_0 --spec-n 0` returns
  `[RESULT]` and leaves port 8299 dead.

## Stage 2 — six E1 runs (isolated server :8299; live unit untouched)

Common argv: live GGUF · `--window 262144 --ts 27,21 --kv q4_0
--cache-ram 8192 --ckpts 4 --mode off --spec-n 2`

| tag | ub | target |
|---|---|---|
| e1-s128 | 128 | 512 |
| e1-s64  | 64  | 512 |
| e1-d128-13k | 128 | 13 100 |
| e1-d64-13k  | 64  | 13 100 |
| e1-d128-42k | 128 | 42 000 |
| e1-d64-42k  | 64  | 42 000 |

Collect `[RESULT]` (prefill + 3x64 decode t/s) + each log's
VITRIOL-FINGERPRINT line.

## Stage 3 — synth-GGUF loadability

Expected llama-arch tensor-set test (derived from
`engine/src/models/llama.cpp`) diffed against `write_synthetic`;
fix until equal; `llama-bench -m synth-tiny` smoke loads and benches.

## Stage 4 — adoption rule

Adopt ub64 in the live config iff it is ≥3% faster at 13.1K and not >1%
slower at 42K. Profile `qwen38-buffedmod-ub64-e1` (copy of live config,
`[kv] ubatch_size = 64`) → `vitriol config load` → unit restart →
fingerprint shows `ub=64`. Rollback: `vitriol config load
qwen38-buffedmod-262k-cache8-mtp2` + restart.

## Stage 5 — record

E1 table + verdict appended to `docs/engine-settings.md`; plan-doc
statuses; one commit per stage.

## Invariants

- I1 arms differ only in `ub=` (fingerprint diff proves it).
- I2 depth = filled `prompt_n` (never window).
- I3 harness isolation: port 8299, own process group, SIGKILL on every
  path; the live unit is never touched by the arms.
- I4 every number carries its fingerprint; no conclusion without one.
- I5 synth fix keeps all lib tests green plus the expected-set test.
