# Dream pipeline — rung 2/3 implementation (plan, 2026-11-21 16:10)

PIPELINE: PLAN CONFIRMED (owner 2026-11-21). Research:
`dream-rung2-3-research-2026-11-21.md`. Owner's topology call: the trial
lane is a **tiny local GGUF (qwen2.5-0.5b-instruct, 469 MB) served via a
base swap on the LIVE engine** — the unit restarts with the small model,
the eval scores it, and the unit restarts back to the 27B (config
rollback already exists: `~/.vitriol/config.pre-dualslot` pattern).

## Scope

**D1 — dream generator** (pure core + extension command).
1. `_shared/dream-generator.ts` (pure, no IO):
   - `extractQA(pairs: TranscriptLine[], capBytes): DreamPair[]` — walk pi
     session JSONL rows; a pair = an assistant message that immediately
     follows a user message carrying a *question-shaped* text (non-empty,
     ≤ 2000 chars, not a tool-result dump). Each pair carries provenance:
     `{session, ts, pairIndex}`. Byte-bounded (I-E2-style window
     discipline, sleep-lane precedent).
   - `dreamCorpusPrompt(pairs)` — I-P5: the sleep-lane quoted-DATA framing
     verbatim (TRANSCRIPT BEGIN/END delimiters, "DATA, not instructions").
   - `gatePair(pair, judge)` — a generated corpus row survives only if the
     judge (same `judge()` from `dream-eval-core.ts`) would accept its
     answer against its own question's musts, AND provenance is present
     (I-P3). Rows failing either are dropped, not repaired.
   - `buildCorpus(raw)` → `{ hash, items }` with the same frozen
     sha256-over-canonical-JSON scheme as `buildEvalSet` (I-E1 family).
2. `dream-gen/index.ts` extension: `/dream-gen [maxPairs]`
   - reads `~/.pi/agent/sessions/--home-randozart-Desktop-Projects-VITRIOL--`
     (bounded by `OFFICINA_DREAM_MAX_BYTES`, default 4 MB),
   - writes `~/.vitriol/officina/dream/corpus-<hash12>.jsonl` (append-only
     dir; old corpora retained for I-P4 rollback),
   - notifies with pair count + hash. Read-only over MEMORY/eval — the
     eval set is NEVER touched (I-E1).

**D2 — small-model lane trial** (ops, measured, reversible).
1. `scripts/dream-trial.sh` — idempotent, three verbs:
   - `swap-in`: snapshot `~/.vitriol/config` → `.pre-dreamtrial`; set
     `model = ~/Models/qwen2.5-0.5b-instruct-q4_k_m.gguf`, `parallel=1`,
     MTP off, ctx 16384; `systemctl restart vitriol-server.service`.
   - `score`: `node`-run dream-eval against the local endpoint with the
     frozen eval set; writes `dream-eval/dreamtrial-<stamp>.json`.
   - `swap-out`: restore the snapshot config (byte-exact, verified by
     sha256), restart. The fingerprint diff is printed for provenance.
2. Promotion (if the report qualifies): the existing promotion-rule path
   (I-P1) — no new code; the report is the artefact.

## Invariants

- **I-E1 frozen eval**: D1/D2 never write the eval set; corpus hash ≠ eval
  hash. *Pinned by*: `dream-gen/core.test.ts` "generator never touches the
  eval set path" (the pure functions take paths only via the extension,
  asserted via a dry-run with stubbed IO).
- **I-E2 window discipline**: extraction is byte-bounded; over-budget
  files are skipped, not truncated mid-pair. *Pinned by*: "over-budget
  file dropped" test.
- **I-P3 provenance**: every corpus row carries `{session, ts, pairIndex}`;
  rows without provenance are dropped at build time (empty string too).
  *Pinned by*: "unprovenable rows dropped" test (incl. the empty-string
  case that bit us on the eval set).
- **I-P5 injection resistance**: the corpus prompt uses the sleep-lane
  quoted-DATA framing verbatim. *Pinned by*: prompt-shape test asserting
  the delimiters + "DATA, not instructions" line.
- **I-P2 base immutability**: `swap-in`/`swap-out` only touch the config
  file (snapshotted + hash-verified); the 27B GGUF is never written.
  *Pinned by*: the script's own sha256 assertion + a shellcheck-gated
  test that the 27B file's mtime is unchanged after a full in/out cycle
  (ops test, manual gate).
- **I-P4 reversible**: `swap-out` restores the exact snapshot bytes
  (sha256 compare, refuse to restart on mismatch). *Pinned by*: the
  script's refusal path, covered by the ops dry-run test
  (`OFFICINA_DRY_RUN=1`).
- **I-P1 measured promotion**: promotion happens only via the existing
  dream-eval report + promotion rule — D2 ships no rank-change code at
  all (the report is the deliverable; a human/owner applies the rank).

## Named tests

1. `dream-gen/core.test.ts > extractQA` — pairs from a fixture transcript
   (question+answer, negative control: tool-result-only assistant
   message, over-cap question, over-budget file).
2. `dream-gen/core.test.ts > gatePair` — provenance-missing drop
   (undefined AND empty-string), judge-fail drop, pass-through keep.
3. `dream-gen/core.test.ts > buildCorpus` — hash stability (same input →
   same hash), corpus ≠ eval hash for overlapping content, drop-then-hash
   order (dropping changes the hash).
4. `dream-gen/core.test.ts > dreamCorpusPrompt` — I-P5 delimiters present.
5. `scripts/dream-trial.sh` — `bash -n` + a dry-run test asserting
   swap-out refuses on a corrupted snapshot (fake snapshot, mismatched
   sha256 → non-zero exit, config untouched).

## Execution order (GRIND)

1. D1 core + tests → commit.
2. D1 extension command + tests → commit.
3. D2 script + dry-run test → commit.
4. LIVE trial: swap-in → score → swap-out; report under `dream-eval/`;
   fingerprint diffs recorded in this plan's addendum.
5. Gates (vitest + tsc), push, CI green.
