# Cloud probe — background admission service (2026-11-14 20:40)

Operator: "I'd actually prefer if Caduceus probes the cloud in the background,
and keeps trying, unless truly everything is exhausted."

Today's cascades were failure-driven: the router learned about cooled lanes,
paused owners and unoffered ids only by burning requests on them. This plan
makes the state **ahead of the request**: one leader session probes candidate
lanes with a 1-token completion and publishes shared facts that every router
already reads.

## Probe contract

```
POST <caduceus>/v1/chat/completions
{ "model": "<owner>/<model>", "messages":[{"role":"user","content":"ping"}],
  "max_tokens": 1, "temperature": 0 }
```

- Authoritative: exercises gateway credentials, upstream availability,
  cooldowns and key circuits.
- Privacy: fixed "ping" payload, never session content.
- Cost: free-tier lanes; token budget 1; bounded cadence and daily cap.
- Results feed the SAME parsers/marks as real failures (`gateway-health`),
  so one leader's probe protects every session.

## Facts (extend `gateway-health.json`)

```
probes: { <laneId>: { kind: ok|cooling|invalid|paused|no_fit,
                      okAt?, failAt?, until?, latencyMs?, failCount, nextProbeAt } }
probeLease: { pid, until }          // single prober across sessions (10 s TTL)
```

- Success: `recordSuccess`, clear owner pause, `kind: ok` (verified TTL 10 min).
- 429: model+owner cooldown via `parseCooldownFromError`; `kind: cooling`.
- 400 key circuit: owner pause + safe bytes via `parseOwnerPause`; `kind: paused`.
- 404: invalid lane + offered list + alias; `kind: invalid`.
- Context/overflow: learned limit; `kind: no_fit` (lane still admitted for
  smaller prompts; fit uses learned windows).

## Scheduler (pure)

- Candidate pool: same free-tier catalog lanes the router uses.
- Skip: invalid lanes, paused owners, active model cooldowns, catalog-hidden.
- Priority: unverified → cooldown newly expired → stale (okAt older than TTL)
  → rotation (stable hash of lane id + tick).
- Per-lane backoff after failures: 30 s → 1 m → 2 m → 5 m → 10 m (cap),
  ±20 % jitter; `nextProbeAt` persisted.
- Global caps: `OFFICINA_PROBE_PER_MIN=6`, `OFFICINA_PROBE_MAX_TOKEN=1`,
  `OFFICINA_PROBE_DAILY=2000`, kill switch `OFFICINA_NO_PROBE=1`.
- **Exhaustion**: every lane invalid or owner-paused, or every cooldown ends
  beyond `OFFICINA_PROBE_EXHAUSTED_MS` (30 min). Then only a slow heartbeat
  (`OFFICINA_PROBE_HEARTBEAT_MS=10 min`) runs to detect recovery. Probing
  resumes automatically on success, catalog change, or new lanes.
- Routing never waits for a probe: probes are async facts; admission uses the
  last known state (P6).

## Leader lease (P1)

Atomic read-modify-write in the facts file:
`acquireProbeLease(pid, ttl=10 s)` — succeeds when unheld, held by us, or
expired. The extension renews while probing and releases on shutdown. Expired
leases are taken over by another session. Prevents N× probe traffic.

## Router preference

- Verified-ok lanes rank before unverified ones (same priority order otherwise).
- Lanes whose probe says invalid/paused/cooling (unexpired) are excluded even
  if not yet marked in provider-health.
- The 2-cloud-retry budget and local fallback stay unchanged: probes make
  retries rare, not impossible.

## Observability

- `/routes` gains a `probe` column: `✓ 120 ms`, `cooling 12 m`, `paused`,
  `invalid`, `unverified`.
- New `/probe` command: leader, lease expiry, verified/total, next probe,
  exhaustion state, per-lane facts.
- New `/doctor`: gateway up, probe leader/summary, engine up/busy, owner
  pauses, budget line. (TUI-local, read-only facts.)
- `lc-probe` harness events (per probe outcome), never route-ledger entries —
  probes must not pollute cost/token statistics.

## Rust core

The scheduler is pure logic and joins `vitriol-core` next to admission:
`probe_delay`, `select_candidate`, `is_exhausted`, `rate_allow` with golden
vectors; TS implementation is authoritative until parity gate I2 passes.

## Invariants

- **P1** One prober across sessions (lease); expired leases are taken over.
- **P2** Probes never carry session content (fixed "ping").
- **P3** A probe failure produces exactly the same marks as a real failed turn.
- **P4** Probing never extends a provider-stated pause (skip paused owners
  until expiry + jitter).
- **P5** Exhaustion = all invalid/paused or cooldowns beyond the horizon; then
  only the slow heartbeat runs.
- **P6** Turn routing never blocks on a probe.
- **P7** Bounded: per-minute, per-day, per-probe token caps; kill switch
  restores today's behaviour.
- **P8** Probe traffic is excluded from route-ledger cost/token stats.

## Files

- `_shared/gateway-health.ts` — probe facts + lease + mark/clear helpers.
- `_shared/probe-scheduler.ts` — pure scheduler + rate limiter.
- `.pi/extensions/cloud-probe/index.ts` — wiring, timer, lease, fetch.
- `resilient-default/index.ts` — verified-first preference.
- `tui/src/tui/routes.rs`, `layout.rs`, `state.rs`, `mod.rs` — probe column,
  `/probe`, `/doctor`.
- `vitriol-core/src/lib.rs` (+ scheduler module) — pure parity functions.
- Tests: scheduler matrix, lease, caps, exhaustion, fake-fetch marks,
  verified-first routing, TUI probe rendering.

## Tests / gates

`npm test`, `npm run typecheck`, `cargo test` (vitriol-core), `cargo test` +
`cargo fmt --check` (TUI), rebuild + install.


## Phase status (2026-11-14 20:25)

- Facts, lease, scheduler, extension, verified-first routing, `/probe`,
  `/doctor`, `/routes` probe column: SHIPPED (`ed5cb34`).
- Rust parity: scheduler functions + **golden vectors**
  (`vitriol-core/testdata/vectors.json`) with TS assertions in
  `officina/.pi/extensions/_shared/core-parity.test.ts` (gate I2).
- Next: JSON-RPC sidecar behind `OFFICINA_ROUTER_CORE=rust`, then flip
  defaults per parity.
