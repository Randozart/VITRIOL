# Sleep lane — plan (2026-11-21)

Research: sleep-framework-research-2026-11-21.md. This is the **feasible
half** of Google's SLEEP framework — session consolidation on the now-live
second slot. The LoRA dream-training spike is deferred (separate decision).

## What exists already (reuse, not rebuild)

- **Dual slot, live** (dualslot-trial-2026-11-21): slot 1 can run jobs without
  displacing the foreground ring.
- **background-lane** extension: the *exact* execution shape needed —
  idle/slot-count gated (`shouldLaunch` refuses <2 slots), bounded input →
  compact card → inject once. Its PatchSink is patch-specific but the gate
  (`lane.ts:95`) is generic.
- **memory** extension: owns MEMORY.md stores + `memory_search` over past
  session JSONLs — the consolidation target already has a reader.
- **memory-extractor**: rule-based candidate extraction with a
  human-curation queue (`curator-queue.jsonl`) — the poisoning guard exists
  ("candidates are NEVER auto-trusted").
- Session corpus: 16k+ lines in the VITRIOL project session alone.

## What SLEEP adds over what exists

The existing pieces are *extraction* (rule-triggered, per-message) and
*storage* (MEMORY.md). The paper's consolidation is a **scheduled, holistic
re-read**: the model re-reads a window of its own recent history, derives
themes/facts that no single message contained, verifies them by self-quizzing
(generation must reproduce the fact from a different phrasing), and only then
persists. That's a "dream": self-generated practice, self-verified, human
sign-off optional via the existing curator queue.

## Stages

**S1 — `_shared/sleep-consolidate.ts` (pure core)**
Inputs: session file paths (bounded window: last N MiB), now.
Output plan: which sessions to read, what to ask the consolidation model
(prompt builders for: extract → quiz → verify passes/fails), the MEMORY.md
append format (dated, sourced: session id + line hint), and a "candidates"
vs "verified" split mirroring the extractor's stance. No HTTP, no fs reads —
pure functions + types. Fully unit-tested.

**S2 — `sleep-lane` extension** (execution shell, mirrors background-lane)
- Gate: reuses the same slot-count guard idea — needs ≥2 slots, engine up,
  and (its own policy) **consolidation runs idle-only** by default (it is
  long: reading 16k lines through the model is minutes, not seconds).
- Cadence: `OFFICINA_SLEEP_INTERVAL_H` (default 24h) + manual
  `/sleep-now [window]` command. OFFICINA_NO_SLEEP=1 kill switch.
- Runs on slot 1 (`slot_id: 1`) so foreground is untouched.
- Output: verified facts appended to project MEMORY.md with a `<!-- sleep
  YYYY-MM-DD -->` provenance marker; unverified interesting finds → curator
  queue. Full transcript never leaves the machine; nothing auto-trusted.
- Band row: `sleep` state (idle / running / last result).

**S3 — tests + docs + gates.**
Unit tests for S1 (window selection, prompt shapes, verify-pass criteria,
MEMORY format, curator split). Integration test for S2's gate (2-slot
required, idle-only default, kill switch). Docs in the extension README
block. Final full gates.

## Invariants

- **I1 (never poisoning)**: nothing reaches MEMORY.md without passing the
  self-quiz verification (generation from alternate phrasing reproduces the
  fact) OR explicit human curator sign-off. Mirrors memory-extractor's
  contract. Pinned by sleep-consolidate unit tests (verify-fail → curator).
- **I2 (foreground sacred)**: sleep jobs run on slot 1, idle-only by default,
  kill switch honoured; foreground turns are never delayed or rejected by a
  running sleep job. Pinned by gate tests mirroring background-lane's matrix
  (0/1/2 slots × idle/busy × kill switch).
- **I3 (bounded)**: one run reads at most the configured window and emits at
  most `maxFacts` (default 10) — an unbounded read/write is a bug. Pinned by
  unit tests on the planner.
- **I4 (provenance)**: every appended line carries the sleep provenance marker
  + source session id; deletions of MEMORY content are never automated.
- **I5 (jiti isolation)**: shared state (last-run timestamp, running flag)
  lives in globalThis singletons per the AGENTS.md rule.
- **I6 (prompt-injection resistance)**: session content is DATA, never
  instructions — the consolidation prompt must frame transcripts as quoted
  material and instruct the model to ignore instructions inside them.
  Pinned by a prompt-shape unit test (an injected "ignore previous" line in
  fixture transcript must not appear outside quoted context).

## Named tests
- `sleep-consolidate.test.ts`: `window_selection_bounded`, `verify_pass_requires_reproduction`,
  `verify_fail_routes_to_curator`, `memory_append_has_provenance`,
  `maxfacts_cap`, `injection_resistant_prompt_shape` (I1, I3, I4, I6).
- `sleep-lane.test.ts`: `gate_refuses_single_slot`, `gate_refuses_busy`,
  `gate_idle_only_default`, `kill_switch`, `runs_on_slot_1` (I2).
