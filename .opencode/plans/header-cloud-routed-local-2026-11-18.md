# "Header says cloud, requests go local" — where that lane is chosen

**Date:** 2026-11-18 18:40
**Mode:** PLAN (research only — nothing changed)
**Owner:** "It's more that the header shows a cloud model, but it tries to route to local."

That reframes it: nothing is being *switched*. The session model is cloud, and a **different layer
chooses the physical lane per request**.

> **SUPERSEDED (2026-11-18 19:30):** the retry-exhaustion theory in §1–§2 below is incomplete. The
> verified root cause — pi emits model changes **only** to the extension bus, so no RPC client ever
> learns of an extension-driven `setModel` and the header goes stale — is in
> `model-header-stale-2026-11-18.md`. Read that first; this file's §3–§5 (pin semantics, invariants)
> still stand.

## 1. Who chooses the lane

`officina/.pi/extensions/resilient-default/index.ts` is a request-level router: it inspects each
request and returns `{ model, thinkingLevel, state }` — the physical model for *that* request — while
pi's session model (what the header renders) stays as selected. Three rules in it produce exactly the
reported behaviour:

```ts
// (1) continuations stick to the previous physical model — whatever it was
if (request.reason === "continuation" && previous && previous.id !== "lapis") { … return previous; }

// (2) a retry walks down the pool …
if (request.reason === "retry") {
  if ((state.cloudRetries ?? 0) >= maxCloudRetries()) {
    // … "After N cloud attempts, local (or one actionable error)"
```

So after enough cloud retries a **cloud-model session is served by the local engine**, and the header
never says so. `setRouteWidget(...)` carries the truth, but it is a strip, not the header.

The ledger agrees: `route-ledger-20261118.jsonl` has 600 entries; the busiest session (331) is
**100 % `llamacpp/Lapis Occultus`** with `reason: "continuation"`. Nothing switched — the lane was
chosen under a session whose header shows what pi selected.

## 2. Two defects, one of them deliberate

| # | defect | verdict |
|---|---|---|
| 1 | **The header reports the *selected* model, never the *served* one.** When they differ, the UI lies by omission. | must fix |
| 2 | **A pinned cloud model can silently degrade to local** (retry rule (2)). Resilience for an unpinned session; a broken promise for a pinned one. | should fix |

Also worth naming: `reason: "continuation"` means the *whole tool loop* stays on whatever lane it
started on, so one downgrade at the top of a turn pins the entire turn to local — which is why it feels
persistent rather than transient.

## 3. Fix

### A. Tell the truth in the header (the one that matters)
Render `selected → served` when they differ, e.g.

```
GLM 5.2 · caduceus            (selected and served agree — today's look)
GLM 5.2 · caduceus → local    (same session, this request went to the engine)
```

Source of truth already exists: `lastDecision` (`tier`, `provider`, `model`, `reason`, `localState`)
is written on every request and published to peers. Consume it in `layout.rs`'s header instead of
`state.model` alone, and keep the header width behaviour the tests already pin (`model_label`).

### B. A pin must not be downgraded silently
- `resilient-default` learns whether the operator pinned a model (the adaptive-router already latches
  `manual`; expose it as a fact peers can read, e.g. the routing registry `role`/`pinned` field, rather
  than duplicating the logic).
- When pinned: **no local fallback** on retry exhaustion — return the actionable error ("cloud lane
  failed after N attempts") instead of quietly serving local.
- When not pinned: keep today's behaviour, and let the operator opt out with
  `OFFICINA_NO_LOCAL_FALLBACK=1` (also a `/route` toggle) for sessions where local is unacceptable.

### C. Record the reason on the surface people read
The ledger already carries `reason` (`user` / `continuation` / `retry`) — surface it in `/routes` so a
downgrade reads as *"retry exhaustion → local"*, not as a mystery.

## 4. Invariants

- **I1 (no silent downgrade when pinned):** with a manual pin, no request is served by `llamacpp`
  unless the operator picked local. Pinned by a unit test on the retry path.
- **I2 (header never contradicts the wire):** when `lastDecision.provider` differs from the selected
  model's provider, the header must show both. Pinned by a render test.
- **I3 (unpinned parity):** with no pin, behaviour is byte-identical to today (same fallback order,
  same reasons). Golden test on the decision function.
- **I4 (virtual models unchanged):** `officina/lapis` keeps routing through the proxy; nothing here
  changes its lanes.

## 5. Files

- `officina/tui/src/tui/layout.rs` — header `selected → served` (reads the existing route facts).
- `officina/.pi/extensions/resilient-default/index.ts` — pinned-aware retry branch, env/toggle.
- `officina/.pi/extensions/_shared/routing-registry.ts` — publish the pin so peers can honour it.
- Tests: retry-with-pin, header-differs, unpinned parity.
