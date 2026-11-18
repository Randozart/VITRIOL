# Why a manual model pick still gets switched

**Date:** 2026-11-18 18:10
**Mode:** PLAN (research only — nothing changed)
**Owner:** "models are still being randomly picked despite explicit manual picks."

## 1. Two different phenomena, one report

The evidence separates them.

**(a) The session model really can change.** `~/.vitriol/officina/routing-registry.d/*.json` shows three
live sessions, all currently `caduceus/freeinference/deepseek-v4-flash` (three TUI pids). The route
ledger for today walks through `aion-3.0-mini`, `agnes-3.0-flash`, `agentrouter/gpt-5.6-sol`,
`claude-opus-4-8`, `claude-opus-5` and local — so *something* is picking models.

**(b) A `officina/lapis` session looks like it changes model every request without changing at all.**
Lapis is a *virtual* model: the pi-side model id stays `officina/lapis` while the Caduceus router proxy
chooses a lane per request (that is what the ledger records, and what `agentrouter/...` entries are).
No amount of pinning fixes that, because nothing is being switched — the session model is stable and
the *served lane* moves. Worth telling apart before fixing the wrong layer.

## 2. The pin has three holes — all in `adaptive-router/index.ts`

```ts
pi.on("session_start", () => { reset(); });          // reset(): manual = false, tier = local

pi.on("model_select", (event) => {
  if (applying || event.source === "restore") return;   // ← HOLE 1
  if (event.model?.api === "pi-virtual" || event.model?.provider === "officina") return;
  if (isLocalModel(event.model)) { tier = "local"; failoverActive = false; return; }  // ← HOLE 2
  if (cloudCandidates.some((m) => sameModel(event.model, m))) { tier = "cloud"; activeCloud = event.model; }
  manual = true;
});
```

and the adoption helper I added yesterday:

```ts
const adoptSelectedLane = (model) => {
  if (!shouldAdoptSelectedLane({ tier, activeCloud, model, candidates: cloudCandidates })) return false;
  ... manual = true;
};
```

| hole | what happens |
|---|---|
| **1. `source: "restore"` returns early** | Resuming a session re-selects the model with `source: "restore"` and the handler bails out — so the restored model never re-latches `manual`. |
| **2. an explicit LOCAL pick does not latch** | `isLocalModel(...)` sets `tier = local` and returns *before* `manual = true`. Picking the engine pins nothing, so the next evaluation may promote to cloud. |
| **3. adoption is candidate-gated** | `adoptSelectedLane` only latches when the live model is in `cloudCandidates`. A resumed model that is *not* currently offered as a candidate (a paid lane, or one that just got cooled) leaves `manual = false`. |

`reset()` on every `session_start` — and the TUI fires that on every session switch/resume — means
holes 1 and 3 are hit constantly: the restored model is adopted only if it happens to be a candidate,
and a local pick is never pinned at all.

## 3. Fix (three small changes, one rule)

**The rule:** *any model the router did not itself apply is the operator's choice, therefore pinned.*

1. `model_select`: drop the `source === "restore"` early return from the **latch** path (keep `applying`
   — that one prevents feedback loops). A restore is a selection; it should pin, just not count as a
   switch (it already cannot, because `switches` is only bumped on the apply path).
2. `isLocalModel`: latch `manual = true` as well. Picking the engine means "stay local"; `/overflow on`
   remains the way back to auto.
3. `shouldAdoptSelectedLane`: replace the candidate test with "is this a cloud-ish model?"
   (`provider !== "llamacpp"` and `provider !== "officina"` and `api !== "pi-virtual"`). `activeCloud`
   then records the lane even if the router would not have offered it — which is exactly what pinning
   means.

## 4. Diagnostics, so this is never guessed at again

- The watchdog/routes surfaces already read the route ledger. Add **one line** distinguishing
  *session model* (pi's) from *last served lane* (ledger). Under `officina/lapis` the first is stable
  and the second moves — read together, they answer "was I switched, or was the lane chosen?". That
  distinction is what this investigation needed and did not have.
- Log every `pi.setModel()` the router performs as `lc-route/switch` with `from → to` and the reason
  (`peer-demotion`, `failover`, `budget`, `cooling`). Today the *reason* exists only inside a turn
  decision that is never persisted.

## 5. Tests

1. **Restore pins:** `model_select` with `source: "restore"` and a cloud model latches `manual`
   (negative control: the current code leaves it false).
2. **Local pick pins:** `model_select` with the engine alias latches `manual`, and a subsequent
   peer-demotion decision does not switch away.
3. **Non-candidate pins:** a resumed model that is not in `cloudCandidates` still latches
   (`shouldAdoptSelectedLane` returns true for any cloud-ish provider).
4. **Virtual stays unmanaged:** `officina/lapis` still does not latch (the proxy owns its lanes).
5. **Manual still yields to `/overflow on`:** after `manual = true`, `/overflow on` clears it (existing
   behaviour, extended to the local case).

## 6. What this does NOT fix (and should not)

`officina/lapis` sessions will keep showing a different *served lane* per request. That is the
resilient default doing its job; the ledger is the record of it. If that is what the owner is seeing,
the fix is a UI label (§4), not routing logic.

## 7. Files

- `.pi/extensions/adaptive-router/index.ts` — the three latch changes.
- `.pi/extensions/adaptive-router/overflow.ts` — `shouldAdoptSelectedLane` predicate.
- tests: `peer-policy.test.ts` / a new `pin.test.ts`.
- optional (stage 2): the session-model vs served-lane line.
