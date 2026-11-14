# Local lane recovery + budget/cooldown policy (2026-11-14 17:30)

Operator report: "Lapis Occultus can't connect."

## Findings (measured)

- Engine healthy: `/health` ok, `/v1/models` reports `Lapis Occultus`, direct
  completions return `OK` — including with a **stale** model id
  (`Qwen3.8-27B-Q3_K_M.gguf`) and a bogus one. The server ignores the id.
- Two sessions started at 13:45/13:52, **before** the engine came up (15:16).
  `llama-cpp-provider` probes `/v1/models` only at startup, so those processes
  never registered the `Lapis Occultus` alias; `resilient-default` resolved
  `local = registry.find("llamacpp","Lapis Occultus")` → undefined and reported
  "engine unavailable".
- Cloud fallback was triply closed: the budget firewall counts **all** cloud
  tokens (free lanes included) → daily 9.07M/2M → `canSpendCloud()` false;
  free lanes cooling (`deepseek-v4-flash` 27,574 s, agentrouter 403 quota,
  dahl 502).
- The failure text hid the real gate ("engine unavailable").

## Decisions (operator)

1. Budget counts **billed usage only** — zero-cost lanes never trip the cap.
2. Free-lane 429 cooldowns **cap at 30 min** (`OFFICINA_LANE_COOLDOWN_MAX_MS`).
3. Self-heal + re-probe + honest error text.

## Implementation

### `_shared/budget.ts`
- Skip rows where `(costEstimate ?? 0) <= 0` when summing daily/monthly tokens.
- `budgetSummaryText` says "billed tokens".

### `_shared/provider-health.ts`
- `recordRetryAfter` clamps to `OFFICINA_LANE_COOLDOWN_MAX_MS` (default
  1_800_000 ms, floor 60 s).

### `resilient-default/index.ts`
- Local resolution: alias first, else any available `llamacpp` model (the
  engine ignores ids, so a stale models.json id is routable).
- Honest failure text: names the actual gate — local model not registered
  (with registered ids), engine down (poll state), billed budget, or the
  earliest lane cooldown.

### `llama-cpp-provider/index.ts`
- Re-probe `/v1/models` while the alias is missing: on `session_start` and a
  bounded backoff timer (≤15 attempts). On success, unregister + re-register
  the provider through the live model registry so the alias appears without a
  restart.

## Invariants

- **I1** Billed-token accounting is unchanged; only zero-cost rows are exempt.
- **I2** Engine up + any `llamacpp` model registered ⇒ lapis selects local.
- **I3** The router error always names the refusing gate (no catch-all).
- **I4** A 429 cooldown never exceeds 30 min.
- **I5** No retry storm when both tiers are unavailable (error is immediate;
  GRIND already pauses on model errors).

## Tests / gates
budget zero-cost exclusion + billed caps; Retry-After clamp (seconds/date);
resilient-default fallback + error text; provider re-probe decision.
`npm test`, `npm run typecheck`; commit (no TUI binary change).
