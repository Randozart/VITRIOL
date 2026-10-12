# State-Size Findings: What the "2.25x Bloat" Actually Is

**Date**: 2026-10-12 (evening, post overshoot-fix)
**Status**: INVESTIGATION COMPLETE - no serialization bug found; one policy
question opened

## Decomposition of the observed numbers

From `/cache` earlier today: RAM entry #1 = 144,008 tok / 6.18 GB
(42.9 KiB/tok), entry #2 = 127,366 tok / 3.03 GB (23.8 KiB/tok), disk
spills consistently 23.5-24.8 KiB/tok.

**Per-token serialization rate = ~23.7 KiB/tok = 1.24x live** (live KV
19.1 KiB/tok). Composition: target attn KV at native q4_0 (~19.1) +
draft f16 KV (~3) + recurrent state + metadata. `state_write_data`
writes tensors in their NATIVE dtype (`llama-kv-cache.cpp:2346`,
`io.write_tensor(k, ...)`) - no dequant round-trip anywhere. This part
is honest and correct.

**The remaining ~2.76 GB in entry #1 is the checkpoint ring.** RAM entry
`size()` = main state + draft state + SUM of `common_prompt_checkpoint`
blobs (`server-task.h:637`). Each checkpoint stores a PARTIAL_ONLY state
blob (`data_tgt` + `data_dft`) - for a 144k-token session that is
hundreds of MB to GB each, and the ring holds up to `--ctx-checkpoints
4`. Measured: ~690 MB average, ~2.76 GB total for entry #1. These are
real resident bytes, correctly counted against the 12 GiB `cache_ram`
limit.

So: **42.9 = 23.7 (honest per-token) + checkpoint-ring amortization.**
No serialization bug. Correcting the earlier note in the overshoot-fix
report accordingly.

## The policy question

Two checkpoint sources:
1. **Restore snapshot** (`server-context.cpp:1992-2000`): when a restored
   state has an empty ring, the ENTIRE restored range is snapshotted once
   so the ring is "usable". That is a near-full duplicate of the state
   (~GBs) taken on every boot/disk restore.
2. **Periodic checkpoints** (`create_checkpoint`, min step 8192, ring of
   4): in-session rewrite protection.

**Their value dropped today.** The overshoot fix (inner `f5fbc4e44`)
made pure growth - the overwhelmingly common divergence - append without
consulting the ring. What still needs the ring: in-session history
rewrites (overshoot beyond the 3-token ring) and client truncations. For
a rewrite, a ring checkpoint pays only when the avoided re-prefill
exceeds the blob-load cost (copying ~0.7-3.4 GB into ctx); for small
overshoots (e.g. the identical-resend 5-token case) the ring is net
NEGATIVE (GB blob load vs a trivial re-prefill).

## Options (owner decision, not implemented)

| option | saves | cost |
|---|---|---|
| A. drop the restore auto-snapshot (1992-2000) | ~1x state size per restored session (GBs) | rewrite-after-restart -> honest re-prefill (rare, correctness-preserving) |
| B. reduce `--ctx-checkpoints` 4 -> 2 | ~1.4 GB per big session | shallower rewrite protection (config change + fingerprint; 0 is FORBIDDEN, 2 is fine) |
| C. cap checkpoint blob size (skip checkpoints above N MiB) | bounded worst case | new code + policy knob |
| D. nothing - it is honest RAM-for-resume | - | cram pressure stays; 2 big sessions near-fill 12 GiB |

Recommendation: A + B. Combined they would take entry #1 from 6.18 GB to
~3.6 GB, fit a third session in cram, and remove the main tier-miss
pressure. The ring keeps its remaining legitimate role (in-session
rewrites, 2 deep) at half the cost.

## Related correction

`.opencode/plans/restored-state-overshoot-fix-2026-10-12.md` says
"serialization bloat 2.25x" - superseded by this document: per-token
rate is 1.24x; the rest was checkpoint accounting.
