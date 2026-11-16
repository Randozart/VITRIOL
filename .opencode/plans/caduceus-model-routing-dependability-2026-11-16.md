# Caduceus model routing: "just pick one and run with it" (2026-11-16)

Owner: *"I really want Caduceus to just pick me a model and run with it so I
don't end up having to resume sessions so much."*

The trace that prompted this:

```
✗ model error: 429: {"message":"[agnes/agnes-3.0-flash] [429]: You have used up
   today's text quota … (reset after 2m 57s)","retry_after":177,
   "reset_at":"2026-11-16T18:54:18.506Z"}
🜍 retry #1
✗ model error: All credentials for model aion-labs/aion-3.0-mini are cooling down
🜍 retry settled
☿ Resume
✗ model error: [agnes/agnes-3.0-flash] [429] … (reset after 1m 33s)
🜍 retry #1
```

## Trace: who does what today

| step | where | behaviour |
|---|---|---|
| gateway picks an upstream lane | Caduceus (OmniRoute, `127.0.0.1:20128`) | **does** reroute per request — the retry hit a *different* lane (`aion-labs`) |
| gateway reports the failure | same | returns `429` with machine-readable `retry_after` and `reset_at` in the body |
| pi retries | `auto_retry_start` / `auto_retry_end` events, consumed at `officina/tui/src/tui/mod.rs:4247-4259` | one extra attempt, then `retry settled` (gives up) |
| Officina's router reacts | `officina/.pi/extensions/adaptive-router/index.ts` | **inert on this session** — see below |
| Officina's TUI reacts | `handle_provider_failure` (`mod.rs:3252`) | **RateLimit falls through to `_ => notice only`** (`mod.rs:3361`) |

### Why the router did nothing

```ts
// adaptive-router/index.ts:270-275 (before_agent_start)
if (ctx.model?.api === "pi-virtual" || ctx.model?.provider === "officina") {
  return undefined;            // lapis owns its own routing — do not fight it
}
...
// :497 (after_provider_response)
if (tier !== "cloud" || !activeCloud) return;     // ← never true on lapis
if (event.status === 429) { /* honour retry-after, cool the lane */ }
```

On `officina/lapis` the router never leaves `tier: "local"`, so the whole 429
cooling block — which *does* honour `Retry-After` and clamps 1 m…24 h — never
runs. Lane parking is therefore unowned on the exact path the owner uses.

### Why the TUI did nothing

```rust
// mod.rs:3267-3365
match failure {
    EngineDown | NoPhysicalModel => { /* start/restart the engine (lapis only) */ }
    ModelNotFound => { /* switch to lapis + replay the prompt */ }
    _ => state.notice = Some((detail, "warning")),   // ← Auth, RateLimit, everything else
}
```

`classify_error` already labels this `Failure::RateLimit` (`heal.rs:41-48`,
tested at `heal.rs:408`), but the action is "show a notice". The turn is dead;
the operator resumes by hand. That is precisely the "resume sessions so much"
symptom.

### A facility that exists and is unused

The gateway publishes per-lane health (`load_gateway_health()`, consumed only by
`/doctor`): each probe has `kind: ok | cooling`, plus `until`, and there are
`pauses` and `invalid` sets (`mod.rs:2036-2056`). Nothing in model selection
reads it.

## Root cause

Routing knowledge is split three ways and no layer owns the whole decision:

1. the gateway knows lane health and quota resets, but treats the client's model
   id as the answer and gives up when its own candidates are parked;
2. the router knows cooling + `retry_after`, but is disabled for the virtual
   model — i.e. on the session the owner actually runs;
3. the TUI knows how to switch model and replay a prompt, but only wires that to
   `ModelNotFound`.

## Plan (three layers, ordered by leverage)

### L1 — TUI: a rate-limit is a switchable failure, not a notice

`mod.rs::handle_provider_failure`:

- Parse the quota hint out of the detail (`retry_after` seconds, or `reset_at`
  as ISO-8601 — the gateway already puts both in the message).
- If the active model is **not** `officina/lapis`: `SetModel{officina/lapis}` +
  `pending_replay` — the exact path `ModelNotFound` already takes
  (`mod.rs:3327-3354`). The turn continues on local instead of dying.
- If already on lapis: schedule `pending_replay` at
  `min(reset_at, now + cap)` (cap from policy, default ~120 s), bounded by the
  existing `can_attempt`/`attempts` window; if the budget is spent, degrade with
  a notice that names the lane and the reset time.
- Keep `failure_seen` dedupe so a burst of 429s does not stack replays.

Effect: **no dead turns from quota** — worst case it resumes on the local engine
by itself.

### L2 — Router: cooling must engage on the virtual lane too

- Move the `after_provider_response` bookkeeping above the `tier !== "cloud"`
  early return (record failures for whatever model actually served), or drop
  the early return and let `activeCloud == null` mean "cool by provider/model
  from the event payload".
- Accept the quota hint from the **body** as well as the headers (the gateway
  emits `retry_after`/`reset_at` in JSON; `Retry-After` may be absent).
- Keep `pickCloud`'s existing `cooling()` filter — it is correct, it just never
  had data on this path.

Effect: the router stops selecting a lane it knows is parked, and the
`/doctor` cooling counters match reality.

### L3 — Caduceus: reroute until something answers (the actual ask)

In the gateway, for a request on a virtual/alias model id:

1. Build the candidate list from all *usable* lanes (not cooling, not paused,
   not invalid) — the health facts already exist.
2. Try them in order with a **per-attempt deadline**; on 429/403/404/5xx **or a
   deadline expiry**, park that lane until its `reset_at` and continue.
3. Only surface an error when every candidate is parked; say how many were
   tried and when the earliest comes back.
4. Record the decision (lane, reroute count, final status) in the route ledger
   and expose it in the lane-health facts, so a silent reroute is still
   auditable.

Effect: "pick a model and run with it" — with 22 connections configured, a
single exhausted quota should never end a turn.

## Invariants

- **I30 — no unbounded retry.** Every ladder has a candidate budget and a total
  deadline; exceeding either surfaces an error naming the attempts.
- **I31 — a parked lane is never selected.** `until > now` ⇒ not a candidate, in
  both router and gateway (pinned by a unit test over a synthetic cooling set).
- **I32 — a reroute is never silent.** The ledger records lane + reroute count;
  the UI shows at most one line ("served by X after 2 reroutes").
- **I33 — parity when healthy.** With no failures, selection is byte-identical
  to today (no extra probe, no extra latency).
- **I34 — local is the floor.** When every cloud candidate is parked, the turn
  continues on `officina/lapis` if it is up; only then may it fail.

## Tests

- TUI: `429` with `retry_after` ⇒ model switches to lapis and a replay is
  scheduled; `429` while already on lapis ⇒ replay scheduled at the hint (capped)
  and never more than one per window; budget exhaustion ⇒ degrade notice.
- Router: a 429 body-hint parks the lane for the stated duration; `pickCloud`
  skips it; the same event on a lapis session still records the failure.
- Gateway: an end-to-end script that serves a model whose only lane returns 429
  and asserts the request lands on another lane (and that the ledger says so).
- Regression: the existing `decision`/`router` matrices must stay green.

## Open questions for the owner

1. When the cloud ladder is fully parked, may it drop to **local** even for a
   long-context turn (slower, but finishes)? I assume yes.
2. How many silent reroutes before you want to know? Current plan: one line.
3. Should a **pinned** model be overrulable for one request, or only a
   virtual/alias id? (Pinning a concrete lane and being told "no" is arguably
   correct; pinning `officina/lapis` must never fail.)

## Risk / rollback

Display and routing only; no schema or session format change. Each layer is
independent — L1 alone removes the "resume by hand" symptom and can ship first.
