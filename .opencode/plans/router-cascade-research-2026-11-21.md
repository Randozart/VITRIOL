# Router-unavailable cascade — research notes (2026-11-21)

Symptom chain (operator report):

```
✗ model error: 503 router_unavailable "all 2 candidate(s) unavailable —
  earliest lane returns in 57s"
🜍 engine unit active — not restarting (OFFICINA_HEAL_ENGINE_RESTART=1 to allow)
🜍 retry #1 / #2 / #3, "Connection error" ×2
⚠ grind: model call failed — loop paused
☿ Resume
✗ rpc prompt failed: Agent is already processing. Specify streamingBehavior
✗ model error: All credentials for model aion-labs/aion-3.0-mini are cooling down
```

## Component map (evidence)

| piece | where |
|---|---|
| Caduceus router (20129) | `officina/scripts/caduceus-router.mjs` (node, pid 868) — NOT systemd (unit `vitriol-caduceus-router.service` inactive/failed; it runs as a bare process) |
| gateway (20128) | omniroute v16.4.2, `~/.vitriol/vendor/omniroute`, pid 1542 |
| park state | `~/.vitriol/officina/caduceus-router-state.json` |
| retry banners | TUI `auto_retry_start` handler → `officina/tui/src/tui/mod.rs:4510-4516` |
| heal gate | `mod.rs:3456`, `heal.rs:238` (`OFFICINA_HEAL_ENGINE_RESTART`) |
| busy error | pi `agent-session.js:1682-1685` — `prompt()` throws when `isStreaming` and no `streamingBehavior` |
| auto-continue | `mod.rs:726-740` — on `compaction_end` sends `CONTINUE_PROMPT` **without** `streaming_behavior` |

## Finding 1 — the 503 is CORRECT behaviour, but the park table poisons itself

Router walk (`caduceus-router.mjs:330-370`): candidates = requested model +
fallbacks minus parked. When all are parked → 503 with `giveUpMessage`
(`caduceus-router-lib.mjs:100-108`). Live state at inspection:

```json
{"parked": {"cfp/openai/gpt-oss-120b": <t+40s>, "?": <t+79s>},
 "reasons": {"cfp/openai/gpt-oss-120b": "HTTP 502", "?": "HTTP 401"}}
```

**Key defect — the `"?"` lane**: `park(state, model ?? "?", …)` at
`caduceus-router.mjs:414`/`437`. When a request arrives with NO model field
(catalog reads, health probes, embedding calls, or the TUI's stats/state RPCs
proxied through), a failure parks the literal key `"?"` — and
`candidateOrder` (`caduceus-router-lib.mjs:80`) only filters by exact model
id, so `"?"` never blocks a real lane... BUT in `giveUpMessage` the `"?"`
entry counts as a parked "lane" and its 60s is reported as "earliest lane
returns in Ns". Worse: a 401 from the gateway (auth blip) parks for 60s and
the operator-visible message blames "candidates", hiding that the REAL
candidates may be fine.

**Second defect — quota hints clamp at 24 h but `reset_seconds: 20054`
(≈5.6 h) is honoured verbatim** (`parseQuotaHint`,
`caduceus-router-lib.mjs:22-34`): one 429 from a flaky upstream provider
benches the lane for hours. The "cooling down" message comes from the
gateway's own credential cooldown (mirrored in
`gateway-health.test.ts:246`); the router then parks the whole lane on the
hint. There is no cap shorter than 24 h and no probe-before-expiry (a lane
could recover in minutes; we wait out the full hint).

**Third — no parked-state override path**: nothing in the router exposes
"unpark now" (no DELETE /park, no admin endpoint). The only escape is
editing the state file or restarting the process (which is NOT under systemd
— `vitriol-caduceus-router.service` shows inactive/failed, the router runs
as a bare pid from some earlier launch; `systemctl` output confirms).

## Finding 2 — retry amplification (TUI + grind make it worse)

1. pi auto-retry (`agent-session.js` `_prepareRetry`): exponential backoff,
   emits `auto_retry_start` → TUI prints `retry #N`. Fine.
2. The 503-classified failures map to `heal::Failure::EngineDown`
   (`heal.rs:50` — `"503"` in the classifier), so the heal layer treats a
   ROUTER policy response as an ENGINE outage and checks the engine unit →
   the misleading "engine unit is active — not restarting" banner. The engine
   was never the problem.
3. Grind's loop + the operator's Resume both send fresh `Prompt`s; if any
   lands while pi is still streaming the previous attempt (auto-retry sleep
   is INSIDE the streaming run: `_prepareRetry` sleeps then continues, so
   `isStreaming` stays true across retries), pi throws
   `Agent is already processing` (`agent-session.js:1685`).

## Finding 3 — the resume race is the TUI's auto-continue/queued path

`mod.rs:726-740`: on `compaction_end` the TUI sends `CONTINUE_PROMPT` with
`streaming_behavior: None`. pi's `_runAutoCompaction` ends with
`return this.agent.hasQueuedMessages()` — when queued messages exist the run
CONTINUES (agent still streaming) after `compaction_end` fires. The TUI
handler assumes compaction_end ⇒ idle and fires the continue prompt into a
still-streaming agent → the exact thrown error. Same trap for queued_prompts
delivery at `mod.rs:775-790` — guarded by `!state.is_streaming`, but the
auto-continue at :730 is NOT guarded by `is_streaming`.

## Fix directions (for the plan)

- **R1 (router)**: stop parking `"?"`; a no-model request failure must not
  enter the park table at all. File: `caduceus-router.mjs` catch/status paths.
- **R2 (router)**: cap quota-hint parks (e.g. min(hint, 15 min)) + optional
  probe-before-expiry; expose `DELETE /park` or `/park?model=X` admin route
  and surface park state in `caduceus doctor`.
- **R3 (TUI heal)**: classify `router_unavailable`/`router_exhausted` as
  RouterPolicy (new Failure variant) — never run engine heal for it; banner
  should say "all Caduceus lanes parked, earliest returns in Ns".
- **R4 (TUI auto-continue)**: guard the compaction_end continue prompt with
  `!state.is_streaming` (same guard the queued-delivery path already has),
  or use `streaming_behavior: followUp`.
- **R5 (ops)**: put the router under its systemd unit with Restart=always
  (the unit exists but is failed/inactive — the bare process predates it).
