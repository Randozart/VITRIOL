# Session Log — 2026-10-12

Anchored summary of the day: LoRA workstream opened and closed (shelved),
a live engine incident root-caused and fixed (identical-resend crash +
dead metrics), dashboard telemetry landed (O1-O3), and both open
efficiency questions answered with data. Full details in the linked
reports; this file is the index + outcome record.

## LoRA workstream — CLOSED, verdict SHELVED (owner accepted)

- Question: does a shared adapter improve language-design/systems
  capability on the daily driver? Survey (200 repos): **no language-design
  adapters exist anywhere**; exactly one credible shared 3.8-base
  reasoning LoRA (`rico03` Opus-distill, PEFT r32, Apache-2.0).
- Converter blocker fixed (inner `cd55f1f3a`): linear-attention v-head
  reorder crashed `LoraTorchTensor` on col-side (out_proj) reorders;
  row->B / col->A permutation override, numerically verified (diff 0.0).
  Required transformers>=5 for `qwen3_5`.
- Round 1 (prose A/B, 12 prompts, 400 tok): consistent dense/code-first
  style shift, ~3/12 confident slips (ld1 row-poly misdefinition, sy5
  `x/2 -> x>>1`, sy6 overflow niggle). Speed 26.4 -> 21.9 t/s (-17%).
- Round 2 (compile gate, owner thesis test): 6 hard Rust problems,
  rustc+run+clippy -D warnings, K=3 stderr-feedback repair loop.
  **Greens tied 1/6 vs 1/6 on DIFFERENT problems**; adapter -16% decode,
  +8% tokens, +25% wall. Thesis "faster + compiler-catchable = win"
  fails on its own terms at K=3. Nondeterminism note: temp-0 is NOT
  deterministic under MTP/batching (proven via a discarded run).
- Verdict: shelve. Adapter kept at
  `~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf` (467 MB); live
  config untouched. Reports: `lora-tryA-opus-reasoning-2026-10-12.md`,
  `lora-round2-compile-gate-2026-10-12.md`,
  `lora-round2-results-2026-10-12.md`,
  `lora-workstream-final-2026-10-12.md`.
- Reusable artifact: `scripts/eval-gate/` (compile-gate A/B harness).

## Engine incident — identical-resend crash, FIXED

- Symptom: 4 watchdog SIGKILL bounce loops (13:58-14:30). Any IDENTICAL
  prompt resend (client retry after slow cold start) = full prefix-cache
  hit; the `[TAG_PROMPT_LOGITS]` n_past-- guard demanded a 5-token
  recurrent rollback; GDN ring (`n_rs_seq`=3) refused; fork's
  `common_context_seq_rm` GGML_ABORTed. Captured backtrace:
  `common/common.cpp:1656 ... p0=11, p1=-1`.
- Fix (inner `953784096`): `common_memory::seq_rm` returns status; launch
  path falls back to full-sequence reset (honest re-prefill). Verified
  200/200/200 on the exact repro, one clean re-prefill warning.
- Also fixed in the same commit: **all accumulated /metrics counters
  served 0 forever** - the cached copy only refreshed on the sleeping
  transition (never fires on always-on servers). Now refreshed by a
  1 Hz post-update_slots hook on the task thread. All counters verified
  live (16/16/8/9 within minutes).
- Companion commit (inner `9b11ccb27`): prior-session WIP
  (`restored_covered`) reviewed, validated, committed - checkpoint ring
  tried before forcing full re-process after restores.
- Doctrine: AGENTS.md updated (metrics fix + identical-resend hazard
  entry with the regression signature: health-deaf + SIGABRT ~60 s after
  a cache-hit request).

## Dashboard telemetry O1-O3 (vitriol-tui; Officina untouched)

- O1 acceptance: derived from `/metrics` spec counters (our engine
  serves no `/props` draft object); existing render line lights up.
- O2 VRAM: already shipped (per-GPU GiB + gauge) - verified, no code.
- O3 cache-hit: `cache N%` + `ejected Nk` on the totals line; engine
  counter un-stuck (above).
- `cargo test` 158 passed. Report: `dashboard-o1o2o3-results-2026-10-12.md`.

## Probes — both open questions answered

- **Prefix stability (opencode pattern)**: synthetic append 32/36 reuse,
  delta-only; real aggregates since the 15:15 fix: 2,703,970 cached /
  130,426 reprocessed = **95.4% cache hit** on actual owner traffic.
  opencode prompt shape confirmed cache-friendly; no restructuring work
  needed.
- **First live MTP acceptance numbers**: 2/9 accepted (22%), 0.33
  tok/step on a short reasoning prompt - accumulation starts now; gates
  the depth-eos/FastMTP decision.

## Commits

- Inner (llama.cpp, main): `cd55f1f3a` converter fix; `953784096` crash
  + metrics; `9b11ccb27` checkpoint-ring fallthrough (prior WIP).
- Outer (main): `24af62d` round-1; `03573e2` round-2 + harness;
  `29134d3` workstream final; `5b7c1de` TUI telemetry + reports;
  `81a41f3` docs flag; `4063b4e` doctrine; `907b7fa` probe docs; this
  session log.

## Open / deferred (all data-gated or owner-judgment)

- Depth-eos external-draft bug (suspect `speculative.cpp:1717` mapped-id
  in accept): the one real speed win left (+10% at depth via FastMTP);
  ascensus-eligible; **gated on accumulated acceptance data**.
- Depth-wall VRAM creep (~23 KiB/token on dev0): watch the new VRAM row.
- Watchdog patience (12x5s strikes) vs swap-stall false kills: owner
  judgment; swap-debt amplifier (~23 GB) was manually relieved today.
- O4 `vitriol-tap` proxy (opencode-side telemetry): unstarted, optional.
- Housekeeping: inner-repo dirty `Makefile` (in-source cmake artifact,
  predates session, untouched); foreign llama-server on port 8765
  (Qwen2-VL-2B, CPU-only, not ours) - owner to confirm wanted.
