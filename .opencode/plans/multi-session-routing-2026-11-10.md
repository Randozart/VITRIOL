# Multi-session adaptive routing (2026-11-10)

Supersedes `second-session-cloud-routing-2026-11-10.md` (Option C's
per-launch static tier is the lazy design; this is what we actually want).

Owner ask: **run multiple sessions and have the router adapt per session,
per turn, based on what the engine can actually serve right now** — no
static "this session is a cloud session" flag.

## What the machinery already does (verified)

**Engine side (`engine/tools/server/server-context.cpp`):**
- `parallel=N` allocates N slots at load; each slot is its own KV state.
- A completion request's `id_slot` (default −1) is assigned by:
  1. **LCP similarity** — a free slot whose prompt is a long-common-prefix
     of the incoming prompt wins (the prompt-cache hit path), else
  2. **least-recently-used free slot** — first free in slot order.
- `get_free_slots` excludes slots currently processing; a request to a
  processing slot is **deferred** (task queue), not rejected — the server
  serves up to N concurrent turns and serializes the rest.

**Router side (`officina/.pi/extensions/adaptive-router`):**
- Decides at every turn boundary (`before_agent_start`), per TUI process
  (each TUI = own `pi --mode rpc` = own extension instances, jiti
  isolation).
- `decideTier` already has exactly the primitives needed:
  `localBusy(load)` (processing/deferred/queueWait/stalled/GPU),
  `localDown`, failover, privacy gate, switch cap, dwell, return budget.
- **The gap:** the decision is made by each session *independently* and
  *symmetrically*. With N sessions and 1–N slots, two sessions can both
  decide "local" on the same turn; one gets a slot, the other queues
  (deferred) behind it. The router cannot see "I am session #2 of 3" —
  it has no peer awareness.

## Design: slot-aware adaptive routing

Principle: **every session starts local-first (the stone); the router
demotes the most-recent, smallest sessions to cloud when the engine's
slots are all committed, and promotes them back when a slot frees.**
The engine's own queueing (deferred tasks) remains the safety net — cloud
spill is a *latency* decision, not an *availability* decision.

### P1 — Shared routing registry (new `_shared/routing-registry.ts`)

- `~/.vitriol/officina/routing-registry.json` (mode 600), written by every
  TUI's adaptive-router at each decision:
  `{ ts, tuiPid, sessionId, tier, model, ctxTokens, slotId? }`.
- Entries expire after ~60 s (stale = TUI died; prune on read).
- Read by `decideTier`'s caller: `peers = registry.list().filter(not me)`.
- **jiti isolation contract:** globalThis-backed singleton, with an entry
  in `_shared/jiti-isolation.test.ts` (the standing rule from AGENTS.md).
- No locking needed: single-writer-per-process, last-writer-wins, 60 s
  TTL — a torn write costs one wrong decision, which the next turn fixes.

### P2 — Slot pinning (`llama-cpp-provider`)

**STATUS 2026-11-11: DEFERRED — not implementable from our side.**

Verified before writing code:
- The `llama-cpp-provider` extension only *registers* a provider
  (`baseUrl` + `models`); requests are issued by pi's pinned core provider
  layer, which exposes no `id_slot` hook. Forcing `id_slot` would mean
  patching vendored, pinned pi — out of scope by policy.
- The Rust TUI has **zero** slot awareness today (`grep slots|id_slot` in
  `officina/tui/src` returns only fire-animation hits), so a Rust
  "slot-pin state machine" would be dead code with no consumer.
- The benefit P2 chased is largely already delivered: the engine picks a
  slot by **LCP similarity** first (`server-context.cpp:1791`), so a
  session with the same conversation prefix lands on the same slot and
  its KV is reused. `id_slot` would add explicit stickiness, not new
  capability.

Revisit only if pi exposes per-request `id_slot`, or if we ship a
first-party engine client (then a pin becomes meaningful and testable).
The original design follows, retained for that day.

- The engine already picks slots by LCP/LRU; make that **sticky per
  session**: when a request lands on slot K (visible in `/slots`
  `id_task` correlation or a new `slot_id` echo — engine already returns
  per-slot state), the provider records `session → slot K` and sends
  `id_slot: K` on subsequent requests.
- Payoff: a demoted session returning to local resumes on its own KV
  (no re-prefill); a new session gets the *free* slot by LRU automatically.
- If the engine never echoes slot ids, fallback: LCP already does 90% of
  this (same conversation prefix = same slot). **Verify first**; the echo
  may be unnecessary work.

### P3 — Peer-aware decision (extend `decideTier` in `overflow.ts`)

Add to `DecideInput`: `peers: Peer[]` (from P1). New decision arm, placed
after privacy and failover (both keep priority):

```
freeSlots = parallel - processingSlots          (from /slots poll)
myRank = peers sorted by (age DESC, ctxTokens ASC)  // newest+smallest first
if freeSlots == 0 and myRank < peers.length:      // I am NOT the senior-most
    → cloud (reason: "N local sessions, no free slot — rank r of N")
```

- **No new switch cap needed**: this is a *demotion*, and the existing
  return path (idle ≥ 60 s + small cloud-token delta) already promotes
  back. The maxSwitches cap should count *voluntary* spills only, or be
  raised to N (config `OFFICINA_OVERFLOW_MAX_SWITCHES` stays; demotions
  excluded — 1-line change + test).
- **Seniority heuristic**: the oldest session with the most context is
  the "owner" of the last free slot (its KV is the most expensive to
  evict/re-prefill). New sessions — typically side-agents / sub-tasks —
  are the cheap ones to bounce. This matches how the operator actually
  uses a second session.
- Sub-agent sessions (spawned via subagent launcher) get a one-line env
  `OFFICINA_SESSION_ROLE=subagent` → rank 0 (always demote first). This is
  the *only* per-role signal; everything else is observed, not configured.

### P4 — Slot count awareness

- `parallel` is known: `/props` or `/v1/models`… actually the engine
  already exposes it in `/slots` (array length). `busySlots()` already
  parses it. `freeSlots = slots.length - processingSlots` — zero new
  engine work.

### What stays unchanged

- Single-session behaviour: with 0 peers, P3's new arm never fires →
  **byte-identical decisions to today** (test: golden decision matrix).
- Privacy gate: confidential prompts stay local regardless of rank — they
  *queue* behind the senior session. (Acceptable: they were going to
  queue on the single slot anyway; if the operator objects, P3 can add
  `privacy=confidential → cloud-OK` as an explicit env opt-in, default off.)
- Failover (engine DOWN): unchanged, still bypasses everything.
- Engine config: `parallel` stays 1 **until** the operator wants true
  dual-local. P1–P3 work identically at parallel=1 (one senior local,
  everything else cloud) — i.e. this design *is* the dynamic version of
  Option C, with zero per-launch flags.
- Budget/cooldowns/catalog: reused as-is.

## Why this beats the alternatives

| approach | per-session static? | sees peers? | resumes on own KV? |
|---|---|---|---|
| Option C (session tier flag) | yes | no | n/a |
| Option B (local=cloud override) | yes | no | n/a |
| `--parallel 2` only | n/a | no (engine-queued) | LCP/LRU luck |
| **P1–P4 (this)** | **no** | **yes** | **yes (P2)** |

## Rollout (each commit gated: npm test + selfcheck + canaries)

1. **P1** registry + jiti-isolation test (~150 lines + tests)
2. **P3** `decideTier` peer arm + seniority rank + switch-cap exemption
   (pure policy → pure tests, 10+ cases)
3. **P2** slot pinning (verify engine slot echo first; may be a no-op
   thanks to LCP)
4. **Docs**: `docs/officina-guide.md` section "Running multiple sessions"
   (no flags at all — that's the point; subagent role line included)
5. **Verify parallel=2 headroom** (VRAM at second slot's working depth;
   governor already manages) — separate decision, independent of P1–P3.

## Invariants
- I1: 0 peers ⇒ decisions byte-identical to today (golden matrix test).
- I2: privacy gate runs before the peer arm; confidential never routes
  cloud by default.
- I3: failover (engine down) semantics unchanged.
- I4: registry writes are the only new FS side effect (mode 600, 60 s
  TTL, never contains prompt text — only ids/tokens/tier/model).
- I5: no engine flag changes; `parallel` is a follow-on operator choice.
- I6: subagent demotion is role-based (env from the launcher), not
  content-based.

## Effort
- P1: 1 session. P3: 1 session (policy is pure; tests dominate).
  P2: verify-first (could be ~0 lines). Total: ~2 focused sessions,
  3 commits.

## Open TUI bug (owner-reported 2026-11-11, tracked here)

**Prompt composer wrap**: the prompt area at the bottom correctly GROWS
vertically when text overflows, but the text itself does not WRAP —
long words / unbroken lines spill past the right edge (or are clipped),
while the box height already accounts for the overflow.

Expected fix shape: the composer renderer measures the box height from
wrapped line count (wrap-then-measure), not raw line count; wrapping must
use the same width the input line is rendered at (accounting for the
prompt glyph prefix). Lives in `officina/tui/src/tui/layout.rs`
(composer render path) — verify with a unit test: a 200-char unbroken
token in a 60-col box renders N visual lines where N ==
ceil(display_width / (cols - prefix_width)) and the box height is N,
not 1.

Priority: after P1–P3 (visual, not routing). Same TUI surface as the
transition-table work, so a regression test slots into the existing
`layout`/`markdown` test modules.
