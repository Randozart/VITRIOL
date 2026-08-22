# REBIS Gateway v2 — single-endpoint dual-model brain

**Date:** 2026-08-22 08:30
**Status:** implementing
**Vision (user):** point hermes-agent at ONE endpoint; Rebis works in pure
unison — Qwen3.8's intelligence with Mellum2's speed. "Mellum and Qwen in a
trenchcoat."

## Alchemical port topology

| head | element | number | port |
|---|---|---|---|
| Qwen3.8 | Sol — gold, Aurum | 79 | **8279** (unchanged) |
| Mellum2 | Luna — silver, Argentum | 47 | **8247** (migrated from 8287) |
| Gateway | Mercury — Hermes, Hydrargyrum | 80 | **8280** (was 8090) |

Au(79) and Hg(80) are period-table neighbors: the gateway beside its solar
head; Hermes-the-messenger as mercury uniting fixed and volatile.

## Architecture

```
hermes ──► :8280  one advertised model: "rebis"
      TURN CLASSIFIER
      ├─ escape hatch model ids rebis-qwen/rebis-mellum → force head
      ├─ tools attached + task kickoff (no tool calls yet) ─► QWEN
      │   (planner authors first calls; E2 failure class caught here)
      ├─ last msg is tool result (executor continuation) ───► MELLUM fast path
      ├─ assistant finalizing after tool work ─► PIPELINE (draft-audit)
      ├─ no-tools plain chat: long/complex ► QWEN ; short ► MELLUM
      └─ low confidence ──────────────────────────────► QWEN (safe default)

PIPELINE (draft-audit):
  Mellum drafts (thinking off for replace-style precision)
  one-shot anticipatio warm: stable_prefix + draft → Qwen cache
  Qwen constrained verdict {complete, accurate, missing[], correction?}
  pass ──► ship draft          fail ──► ship Qwen correction

Route decisions logged to distill store (router tuning data).
Misroute safety: uncertain ⇒ Qwen.
```

- Streaming: buffered complete responses; SSE synthesized when client sets
  stream:true. True incremental stream-warming documented as follow-up.
- Audit policy: finals + flagged turns audited; mid-flight executor turns
  schema-validated only. Latency profile documented.
- Advertised context: 65536 (Luna binds). Escape hatches via model id suffix.
- Old `steer`/`passthrough` modes kept behind --mode.

## Migration list

1. opencode.jsonc: mellum-think provider :8287 → :8247
2. hermes config.yaml: VITRIOL-MELLUM + SHIM entries → :8247 / :8280,
   advertise `rebis`
3. shim launch commands/docs: port 8090 → 8280

## Implementation units

1. Route classifier (pure fn + selftest decision table)
2. Pipeline turn handler (draft → warm → verdict → correct) w/ OAI
   tool-call translation on corrections
3. /v1/models rewrite advertising `rebis` (+ per-head ids)
4. Port migration (defaults 8247/8280) + config file updates
5. Selftests + three-route integration test vs live servers
6. Battery validation: S1-style task through hermes→gateway; compare vs
   recorded baselines (A: 18m23s direct / B: 130s loop)

## Progress log

- 2026-08-22 08:35 — plan written; implementing.
- 2026-08-22 09:10 — **Gateway v2 LIVE.** Route ladder + pipeline + models
  rewrite implemented and selftested (classifier table, SSE stitcher, models
  synthesis). Three-route integration verified against live servers:
  kickoff→Sol 23.6s · executor→Luna 5.1s · finalizing→pipeline 11.8s with
  native tool_calls shipped. Configs migrated (opencode :8247; hermes
  REBIS-GATEWAY :8280 advertising `rebis`). Distill records flow per turn.
- Known v1 warts: Luna's no-think drafts leak planning narration into prose
  (audit layer compensates); incremental stream-warming deferred to follow-up
  (one-shot warm per audited turn today).
- Next: hermes end-to-end run against :8280; route-tuning from distill data.

## Progress log (cont.)

- 2026-08-22 11:30 — **TUI REBIS tab live** (commit e0bffb8): head status
  cells (Mercury/Sol/Luna + model + velocity), route distribution, audit
  ledger (PASS/FAIL/corrections/compactions), scrolling event stream from
  shim-events.jsonl. Luna first-class in config/poller (VITRIOL_LUNA_PORT,
  heartbeat tok/s). Verified live: event stream advancing with real traffic.
- Known wart found while navigating: Officina REPL input focus blocks forward
  Tab cycling past OFFICINA — BTab wrap-around reaches REBIS. Keymap fix
  deferred.
- Boot convenience still pending: scripts/rebis-gateway.sh exists;
  `rebis` one-command mode in rebis-servers.sh sketched but not finalized.
