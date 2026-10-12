# Incident + O1-O3: Identical-Resend Crash, Dead Metrics, Dashboard Telemetry

**Date**: 2026-10-12 (session ~13:45-15:20 CEST)
**Status**: FIXED + VERIFIED; O1/O2/O3 dashboard telemetry landed
**Commits**: inner `953784096` (engine fixes); outer this commit (TUI + docs)

## Incident: engine bounce loop (13:58-14:30)

The daily driver wedge-and-restarted four times (557614 -> 639451 ->
649042 -> 660170, each SIGKILLed by the autosave hang-watchdog after
~60 s health-deaf). Root cause chain, fully traced:

1. **Trigger**: an identical prompt resend (client retry after a slow
   cold-start response) produces a FULL prompt-cache prefix hit.
2. The `[TAG_PROMPT_LOGITS]` guard (`server-context.cpp:3742`) decrements
   n_past (12 -> 11) so at least one token is re-evaluated.
3. The recurrent (GDN) half of the hybrid memory must roll its state back
   from the last decoded position (15) to 10 - a 5-token rewind. The
   per-token snapshot ring is `n_rs_seq`-bounded (3), refuses the rollback
   (`llama-memory-recurrent.cpp:204`), and `llama_memory_hybrid::seq_rm`
   propagates false.
4. Our `common_context_seq_rm` treated false as fatal (`GGML_ABORT`,
   `common/common.cpp:1656`). SIGABRT; while `ggml_print_backtrace`
   spawned gdb on itself the accept loop stayed deaf, and the watchdog
   force-restarted the unit. systemd logs per generation: ~12.5 G swap
   peak, `stop-sigterm timed out`.
5. Amplifier: ~23-28 GB of swap debt on the box (zram nearly full) made
   cold requests slow enough that retries (step 1) were likely.

Evidence: captured ggml backtrace in the gen log (line
`common/common.cpp:1656: failed to remove sequence 0 with p0=11, p1=-1`),
deterministic repro A,A',A'' (200, then health-deaf at the identical
resend; different prompts fine).

Note: an unrelated uncommitted WIP hunk in `server-context.cpp`
(`restored_covered`, checkpoint-ring fallthrough for the restore path,
~3652-3680) belongs to another workstream. It is compiled into the
running binary but deliberately EXCLUDED from the commit (staged by
hunk). Owner should reconcile it.

## Fixes (inner `953784096`)

1. `common_memory::seq_rm` returns the removal status (no abort); the
   launch path falls back to `seq_rm(id, -1, -1)` + `keep_first(0)` =
   honest full re-prefill (~12 tokens here) instead of dying. Verified:
   identical-resend triple 200/200/200, exactly one
   `full re-prefill` warning, health OK throughout.
2. The cached `/metrics` copy was only refreshed on the sleeping-state
   transition - which never fires on an always-on server - so **every
   accumulated counter served 0 forever** (only live-read gauges like
   `kv_ejected_total` moved). Added a throttled 1 Hz post-update_slots
   hook on the task thread; routes refresh `cached_metrics` from it.
   Verified: prompt_tokens_total/cached, tokens_predicted_total,
   n_decode_total all advance; spec_decode counters live.

Side finding: the launch-path seq_rm previously ran on every
PROCESSING_PROMPT iteration; it is now STARTED-only (same effect - the
outer call was a no-op on continuation iterations - but no longer
re-evaluated per chunk).

Provenance: `VITRIOL-FINGERPRINT` 15:15:00 launch = blessed config
unchanged (`ts=27,21 c=262144 kv=q4_0/fa spec=mtp:3 cram=12288 cxr=1
ckpt=8192 ckpts=4 par=1`); binary-only change.

## O1-O3 dashboard telemetry (vitriol-tui)

Target surface: `vitriol-tui` (owner's telemetry surface; Officina
untouched per directive).

- **O1 acceptance**: `/props` has no `draft` object on this engine
  (vestigial poller path) - dashboard now derives the DraftSnapshot from
  the `/metrics` spec counters (`spec_decode_num_{draft,accepted,drafts}
  _tokens_total`); the existing render line (`ui.rs:243`) lights up.
  Live: 9 drafted / 2 accepted / 6 steps.
- **O2 VRAM**: already shipped (per-GPU `vram_used/total` + gauge,
  `ui.rs:403/682`) - verified, no code needed.
- **O3 cache-hit**: engine side fixed (above); TUI adds
  `prompt_tokens_cached_total` + `kv_ejected_total` to MetricsTotals and
  renders `cache N%` + `ejected Nk` on the totals line.
- Bonus: per-position acceptance series parse-guarded against label
  pollution (test).

Tests: `cargo test --release` 158 passed. Harness: `scripts/eval-gate/`
from round 2 untouched.

## What the new telemetry already tells us

First live MTP numbers off the dashboard path: 2/9 accepted (22%),
0.33 tok/step on a short reasoning prompt - n=3 drafts mostly wasted on
this prompt class. Long-session acceptance data now accumulates for the
depth-eos/FastMTP decision.

## Deferred / flagged

- The `restored_covered` WIP hunk was reviewed and committed as inner
  `9b11ccb27` (checkpoint-ring fallthrough after restores; validated
  under load, self-contained against HEAD infra).
- Swap debt (~23 GB) + watchdog patience: the amplifier remains; options
  were presented (close idle clients / cache_ram trim / MemoryLow /
  watchdog strikes) - owner chose manual cleanup this time. Revisit if
  cold-start stalls recur.
- O4 `vitriol-tap` proxy (opencode-side telemetry): still available,
  unstarted.
