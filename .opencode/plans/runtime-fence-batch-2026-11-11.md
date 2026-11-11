# Runtime-fence batch — 2026-11-11

Follow-up to `officina-multisession-hardening-2026-11-11.md` (all four
stages landed same day). Four verified gaps, two commits. Scope chosen by
operator from a hardening menu ("Batch 1-4").

## Gaps (verified this session)

1. **Runtime session-switch fence missing** — the flock guard fences
   *startup* only. `/resume` modal Enter (`tui/mod.rs:706`) and the
   imported-session path (`tui/mod.rs:1388`) both send
   `RpcCommand::SwitchSession` with no peer check. A peer TUI that
   started before us holds our store; its ACTIVE session file appears in
   our /resume list; switching onto it = two pi writers on one jsonl —
   exactly the corruption the startup fence prevents (I2 reopened at
   runtime).
2. **Manual latch is cosmetic** — `manual` (`adaptive-router/index.ts`)
   gates only the status-line text (line 244). `before_agent_start`
   still applies tiers over the user's hand-picked model next turn, and
   `manual = true` (line 478) never resets except on `session_start`.
3. **Registry shard accumulation** — `routing-registry.d/` already held
   10 files from dead pids; expired shards are never unlinked.
4. **jiti-isolation contract entry missing** for lane-health's
   globalThis cache (AGENTS.md rule: new cross-extension state gets a
   test in `_shared/jiti-isolation.test.ts`).

## Fix plan

### Commit 1 — extensions (TS)
- `adaptive-router/index.ts`: gate the APPLY path on `!manual` — when a
  hand-picked (non-candidate, non-local) model is active, skip the
  `pi.setModel` switch; the pick sticks. `/overflow on` clears `manual`
  (re-arm). Status line already announces "overflow paused".
- `_shared/routing-registry.ts`: `gcExpiredShards(dir, now, ttl)` —
  unlink shard files whose every entry is expired; called fire-and-forget
  from the shard-merge read path (never throws, never blocks reads).
- `_shared/jiti-isolation.test.ts`: lane-health entry — two jiti
  instances share one cached health object after a single fetch.
- Gates: `npm test` + `npx tsc --noEmit` + build-patch canaries.

### Commit 2 — Rust runtime switch fence
- `session_guard.rs`: `live_peer_session_stems()` — read the shard
  registry (`OFFICINA_ROUTING_REGISTRY_DIR` override honored, legacy
  single file included for transition), return fresh (60 s TTL)
  sessionId stems from peers whose `tuiPid != ours`. Pure helper
  `session_held_by_peer(stem, peers)`.
- `tui/mod.rs`: gate BOTH SwitchSession call sites — if the target
  file's stem is held by a live peer, refuse with a Diag notice
  ("session is live in another officina — not switched") instead of
  sending the RPC. Same-store switches by us stay safe: we hold the
  store lock, peers can never acquire it, so only the peer's ACTIVE
  file (identified by registry sessionId) needs the fence.
- Gates: `cargo test` + `cargo fmt --check` + rebuild + install
  `~/.local/bin/officina` (AGENTS.md officina-rework gate).

## Invariants

- **I2 (extended)**: at most one live pi writer per session file — at
  startup AND at runtime switch paths; pinned by the peer-stem gate
  tests (`live_peer_session_stems`, `session_held_by_peer`).
- **Manual respect**: routing never overrides an explicit user model
  selection until `/overflow on` (or a new session); pinned by the
  adaptive-router apply-path test.
- **I7 unchanged**: shard GC only removes shards whose entries are ALL
  expired; pinned by the gc test (live shard survives, expired vanishes).
- **jiti contracts**: lane-health cache shared across instances; pinned
  in `jiti-isolation.test.ts`.

## Deferred (explicit)

- `VITRIOL_CACHE_KEEP=1` engine experiment (needs a restart window).
- Legacy `routing-registry.json` deletion (transition week not over).
- Feature backlog: vitriol-router proxy, great-harness Phase 6, standing
  TODO disposition, mtp2/mtp3 A/B.
