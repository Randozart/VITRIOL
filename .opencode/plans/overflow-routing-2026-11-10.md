# Overflow routing — local-first, cloud spillover (2026-11-10)

Status: MVP IMPLEMENTED 2026-11-10 12:48.

MVP = `.pi/extensions/adaptive-router` (Officina). The shared `vitriol-router`
proxy remains the follow-up for opencode/subagents.

Cloud lane selected because live OmniRoute availability was checked:
- `opencode-go/deepseek-v4-flash` → 429 cooldown (earlier)
- `glm/*` → weekly/monthly quota exhausted
- `opencode-go/deepseek-v4.1-flash` → 200, tool calls work, 1M ctx (selected)
- `ddgw/claude-haiku-4-5` → 200, 200k ctx (fallback candidate)

Implemented:
- engine snapshot: recent queue wait + processing/deferred
- pure policy + 13 tests (idle/busy spill, privacy, dwell, return budget, switch cap)
- wiring tests: local busy → cloud `setModel`, idle → local, sensitive → local
- auto-route defers when overflow is armed
- below-editor route status (prioritized over other indicators)
- 5-minute cloud cooldown on 429/5xx, 60s on error completion
- manual model selection pauses routing
- session-panel suppresses the false "model mismatch" warning while overflow is armed
- live probe: local busy → `setModel` cloud → assistant replied from
  `opencode-go/deepseek-v4.1-flash`; widget `route cloud · sw 1 · 0k tok`

Restart Officina to load the extension (the running pi process predates it).

## Native OmniRoute free-catalog discovery (2026-11-10 13:42)
- `OFFICINA_OVERFLOW_CLOUD=free` now means: read OmniRoute's cached catalog
  (`~/.vitriol/officina/omniroute-models.json`) and auto-discover tool-capable
  free lanes. No hand-maintained model list.
- Catalog filter: chat + `capabilities.tool_calling` + explicit zero pricing
  (or known-costless `opencode-go` / `cloudflare-playground`) + chat context.
- Owner order: `opencode-go` → `glm` → `zai` → `cloudflare-playground`
  (duckduckgo-web and aihorde are free but have no tool calling, so excluded).
- Effort-tier duplicates (`-high/-low/-max/-medium/-minimal/-none/-xhigh`) are
  dropped when a base model exists; max 8 per owner, 48 total by default.
- Health: per-model cooldown from `headers.retry-after` (clamped 1m–24h);
  account-wide cooldown for GLM/Z.ai 429s; owner-wide for auth/5xx.
- `/overflow-catalog` lists candidates, context and remaining cooldown.
- Live: discovery picked `opencode-go/deepseek-v4.1-flash`; cloud answered
  successfully. Verified candidates include `glm/glm-4.7-flash` and cfp free
  tool lanes when their providers are healthy.
- Zen free tier remains separate: needs `OPENCODE_API_KEY` (currently unset;
  all `zen/*-free` return 401).

## Failover (2026-11-10 13:50)
- `OFFICINA_OVERFLOW_FAILOVER=1` (default) treats a **DOWN** local engine
  differently from a busy one: spill to the free/cloud candidates immediately,
  ignoring the switch cap and token budget. When the engine comes back and is
  idle for ~5s, failover returns to local without the cache-affinity dwell.
- Pre-first-poll window is optimistic (local), so a healthy engine is not
  diverted to cloud on startup.
- Sensitive/confidential turns still never leave local, even during failover.
- Live probe (bad local endpoint :9): `overflow → cloud: local down — emergency cloud`,
  widget `route cloud · deepseek-v4.1-flash · failover`, cloud replied OK.

## Goal
Keep the local GPU busy with local-affine sessions. When the local engine is
already processing/queued (or a foreign tenant is using the GPU), new turns
spill to a cloud model. Never thrash provider caches.

## Measured basis (live)
- `/metrics`: queue_time_sum 1404.42 s / 543 requests = **2.59 s average**,
  45/531 >2 s.
- Local engine `--parallel 1` → contention is effectively binary.
- Cloud lane available through OmniRoute (`omniroute/*`, tool-capable, 200k–1M ctx).
- Existing auto-route classifies privacy/complexity but **does not switch models**.
- `pi.setModel(model)` + `ctx.modelRegistry` make turn-boundary switching feasible.

## Cache-aware rules
1. Decide only at user-turn boundaries (`before_agent_start`); never per token
   or per tool round.
2. Session stickiness: once on a tier, stay unless the local tier is contended.
3. Minimum dwell on a tier (default 3 turns). Max 2 switches/session.
4. Return to local only when local has been idle (default 60 s) **and** the
   cloud session has accumulated ≤ `RETURN_TOKEN_BUDGET` tokens (default 16k)
   so re-prefill cost stays bounded.
5. Sensitive/confidential turns always local (auto-route classifier).
6. Budget guard: cloud token cap per session; no cloud when exhausted.
7. Respect manual model selection: a user `model_select` disables routing for
   that session.

## Algorithm (MVP)
Inputs per turn: tier, switches, turns-on-tier, cloud tokens, local processing/
deferred/queue-wait/GPU load, privacy, cloud context fit, budget.

```
local_busy = processing>0 || deferred>0 || queueWaitMs>=1500 || stalled
             || gpuLoad>=0.85
if privacy != safe: local
elif tier == local:
    if !local_busy: local
    elif cloud_fits && budget && switches < 2: cloud
    else: local (queue)
else: # cloud
    if !cloud_fits || !budget || switches >= 2: local
    elif turns_on_tier < 3: cloud
    elif local_busy: cloud
    elif local_idle >= 60s && cloud_tokens <= 16k: local
    else: cloud
```

## Configuration
`~/.vitriol/officina/env`:
```
OFFICINA_OVERFLOW=1
OFFICINA_OVERFLOW_CLOUD=free   # auto-discover OmniRoute free tool-capable lanes
OFFICINA_OVERFLOW_QUEUE_MS=1500
OFFICINA_OVERFLOW_FAILOVER=1
OFFICINA_OVERFLOW_MIN_DWELL=3
OFFICINA_OVERFLOW_RETURN_IDLE_MS=60000
OFFICINA_OVERFLOW_RETURN_TOKENS=16000
OFFICINA_OVERFLOW_MAX_SWITCHES=2
OFFICINA_OVERFLOW_CLOUD_BUDGET=2000000
```
Kill: `OFFICINA_NO_OVERFLOW=1` (or unset `OFFICINA_OVERFLOW`).

## Implementation
1. `_shared/engine.ts` + `vitriol-decode/decode.ts`: expose recent queue wait,
   processing/deferred in the shared snapshot.
2. `.pi/extensions/adaptive-router/`:
   - `overflow.ts` pure policy (tier decision + tests).
   - `index.ts` pi wiring: start poller, classify privacy, `pi.setModel`,
     below-editor status widget, `/overflow` command, cloud token accounting,
     manual-switch detection.
3. `auto-route`: defer when overflow is armed (mutual exclusion).
4. Env config + docs.
5. Tests: pure policy matrix; telemetry parse; live probe.

## Status widget
`setWidget("overflow-route", ["route local · q 2.6s · sw 0"])` below editor.
On switch: `route cloud · sw 1 · 0k tok`.

## Follow-up
Shared `vitriol-router` proxy for opencode/subagents: sticky `X-Session-Id`
affinity, local-first, cloud overflow, same cache-aware policy, central budget.
