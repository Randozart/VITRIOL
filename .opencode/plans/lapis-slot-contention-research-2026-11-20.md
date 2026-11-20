# Lapis routing to a busy single-slot engine — research (2026-11-20)

## Findings (traced)

1. **Decision uses a ≤700 ms-stale snapshot.** `_shared/engine.ts:26`
   `DEFAULT_POLL_MS = 700`; `resilient-default/index.ts:608` builds
   `load.processing` from `snap.processing`, the last `/slots` poll. Between
   snapshot and request arrival, a second session can claim the only slot.

2. **`localBusy()` has no slot-capacity term.** `adaptive-router/overflow.ts:122`:
   `processing > 0 || deferred > 0 || queueWait >= cutoff || stalled || gpuLoad
   >= cutoff`. With `slotCount = 1`, `processing = 0` reads "idle" even if a
   request is in flight but not yet reflected in the poll.

3. **`freeSlots` is computed but unused by the base arm.**
   `resilient-default/index.ts:841`:
   `freeSlots: Math.max(0, slotCount - (snap.processing ?? 0))` — consumed only
   by `peerDecision()` (junior-session demotion), never by the local-idle gate.

4. **Local admission failures are invisible to the router.**
   `after_provider_response` (resilient-default:962) starts with
   `if (!activeCloud) return;` — a local 400/503 (slot busy / admission) records
   nothing: no failure mark, no cooldown, no notification.

5. **The local→cloud rescue arm exists but is gated on pi retrying.**
   `:705 if (request.reason === "retry")` → `:746 if (failed &&
   isEngineProvider(failed.provider)) { tier = candidate ? "cloud" : "local" }`.
   pi sets `reason: failed ? "retry" : ...` (bundle chunk-44DLKSKE), so the arm
   only fires when pi re-enters with a failed previous message. pi's streaming
   retry default is `maxRetries: 0` (bundle chunk-PMRZO46Y); a turn whose local
   request is rejected typically ends as `stopReason: "error"` and the next turn
   arrives as `reason: "user"` — the rescue never runs, and the error surfaces
   to the operator as a dead turn.

6. **The dispatch point for a fresh check** is `resilient-default:905
   if (local) { ... return { model: local } }` — the last place a probe can run
   before the request leaves the router.

7. `parseSlots` (`vitriol-decode/decode.ts:154`) already yields per-slot
   `busy` from `is_processing`, so an admission probe is a single GET + parse.

## Proposed fix

- **A. Fresh admission probe before committing to local** (closes the race):
  one `GET /slots` with a short timeout (~250 ms) right before returning the
  local model; if any slot is busy → demote this turn to cloud when a candidate
  and budget exist, else proceed local (fail-open — never block on a probe).
- **B. Record local contention** (makes the next decision honest): extend
  `after_provider_response` to handle the local tier; on an admission-shaped
  failure mark "local contended" for a short window (~3 s) in shared state so
  the next turn's decision sees it instead of a blind snapshot, and surface a
  band/notice instead of a silent dead turn.
- No "one-slot pessimism" hack — A closes the window; C would only raise the
  cloud-spill rate.

## Invariants (draft)

- I1: single-session, engine-idle behaviour byte-identical (parity: absent probe
  input = pre-change decision). Pinned by overflow.test.ts parity matrix.
- I2: a probe failure/timeout never blocks a local request (fail-open).
- I3: cloud demotion by contention does not charge the cache-affinity switch
  cap (it is arbitration, like peer demotion).
- I4: a contended local turn with no cloud candidate still runs locally.
- I5: ledger `localState` gains "contended" only when observed (no fabrication).
