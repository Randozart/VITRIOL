# Slot Fairness Fix — chunk-level continuous batching — 2026-09-27

**Status**: DONE. Inner commit (fair chunking), deployed on live engine.

## Symptom

With one slot running a long prefill (Hermes 190-201K-token sessions), tasks
on the other slot sat at `n_prompt_tokens_processed = 0` for minutes:
- cert request starved 92s+ twice (2026-09-27 optane cert runs)
- owner-reported: "the 2 slots struggle dividing the work"
- dual-slot 1.51x aggregate (decode-heavy bench) never showed it: generation
  adds 1 token/slot/iteration - fair by construction. Starvation is a
  PREFILL-phase property.

## Root cause

`server-context.cpp` update_slots, prompt chunk-add loop:

```cpp
while (slot.prompt.n_tokens() < slot.task->n_tokens() && batch.size() < n_batch) {
```

`n_batch` defaults to 2048 (engine sets only `--ubatch-size 128`). One slot's
fill loop consumes the entire logical batch every iteration; the other slot
hits the `batch.size() >= n_batch` skip in `iterate(slots, ...)` and gets
zero tokens until the first prompt completes. A 190K prompt = ~92 consecutive
monopolized iterations.

## Fix

Cap each slot's per-iteration prompt chunk at `n_ubatch` (128):

```cpp
while (... && batch.size() < n_batch && batch.size() - n_tokens_prev < n_ubatch) {
```

Generation phase untouched (already 1 token/slot); mtmd (non-splittable) and
alora branches untouched; decode() sub-ubatch pipelining untouched.

## Measured (dual-3060, qwen3.8-27B 262K, 2026-09-27, cleared server)

| test | before | after |
|---|---|---|
| small 619-token request on slot 1 while slot 0 prefills 40K | **110s wall** (107s starvation) | **3s wall** (3.3s own prefill) |
| solo prefill t/s (7743 tok, slot 1) | 492.9 | 435.4 (−11.6%) |
| large prefill 51,483 tok (interleaved w/ small req) | - | 357.9 t/s |

Cost accepted: −11.6% solo prefill throughput for fair latency. Always-on,
no flag (correct continuous-batching semantics; contended baseline under
Hermes decode load was 75.2 t/s, so the absolute cost is smaller than the
idle-to-idle delta suggests).

## Impact

- Interactive latency during mega-prefills: seconds, not minutes
- Demiurge M2 unblocked: pressure signals (deferred tasks, queue wait) are
  now honest - starvation and load are distinguishable
- Fingerprint: unchanged (code-only change, no flag deltas)
