# Officina convenience harness — full plan (2026-11-10 16:10)

Status: implemented P0/P1 core 2026-11-10 16:12.

Delivered:
- route ledger + `vitriol caduceus stats`
- persistent provider health/cooldowns
- profiles `auto|fast|deep|local`
- free token budget (2M/day, 40M/month defaults)
- preflight CLI + sidebar readiness rows
- TUI commands `/profile`, `/why`, `/free`, `/cost`, `/keys`, `/providers`
- free provider coverage expanded to all zero-priced tool lanes
- Dahl keepalive timer
- `accounts enable --present`, `import-keys`, `keys`
Remaining: shared proxy for opencode/subagents, model-picker badges, keyring.

### Live validation 16:12
- Forced engine-down turn routed to `caduceus/opencode-go/deepseek-v4.1-flash`
  and answered; ledger recorded 1 cloud request/3264 tokens, health recorded
  success; `/free`, `/providers`, `/keys`, `/cost`, `/profile` commands work.
- `vitriol caduceus preflight --json`: local/gateway/keys/catalog/keepalive OK.
- Tests: 702 JS, 99 Rust, selfcheck PASS.

## Invariants
- Automatic routing is **free/zero-cost only**. `OFFICINA_PAID_ALLOWED=0`.
- Local-first: VITRIOL → free cloud overflow → failover when local is down.
- No automated signup, no multi-account farming.
- No boot enable of the local engine without explicit operator action.
- Never persist prompts or API keys in telemetry.

## Phase P0 — visibility + persistence

### Route ledger (`_shared/route-ledger.ts`)
- JSONL at `~/.vitriol/officina/route-ledger.jsonl`, mode 600, daily files
  (`route-ledger-YYYYMMDD.jsonl`) with 30-day retention.
- GlobalThis singleton for jiti isolation.
- Entry: `ts, sessionId, reason, tier, provider, model, owner, localState,
  candidatesSkipped, latencyMs, tokensIn, tokensOut, cacheRead, cacheWrite,
  costEstimate, outcome, errorClass`.
- Hooked from `resilient-default` + `adaptive-router` on route, response,
  retry, and errors.
- `vitriol caduceus stats [--today|--month|--json]`.

### Persistent provider health (`_shared/provider-health.ts`)
- `~/.vitriol/officina/provider-health.json` mode 600.
- Per model: `lastSuccess, lastFailure, status, cooldownUntil, retryAfter,
  errorClass, successCount, failureCount`.
- Owner-level entries for 401/403/5xx and GLM/Z.ai 429.
- Loaded at extension init; expired entries pruned; survives restarts.
- Health rules:
  - 404/410 → model 1h
  - 401/403 → owner 1h
  - 429 → `retry-after` or 30m; owner-wide for GLM/Z.ai, model-wide elsewhere
  - 5xx/network → 5–60m
  - success clears model cooldown.

### Preflight
- `vitriol caduceus preflight [--json]` and TUI startup banner:
  `local up/down · cloud N keys/M candidates · profile auto · Dahl 0d`.
- Checks: engine, gateway, keys, Dahl keepalive, catalog age, top cooldowns.
- Optional session-only engine start via polkit; never boot-enable.
- TUI shows a compact banner and a cloud-only warning when local is down.

### TUI commands
- `/why` — last route decision, candidates skipped, cooldown reason.
- `/free` — free candidates with provider, context, tools, cooldown.
- `/cost` — session/today/month tokens, local vs cloud, estimated savings.
- `/providers` — key presence, gateway link, health/quota.
- `/keys` — masked key status (source: env-file/shell).

## Phase P1 — profiles + budget + convenience

### Profiles
- Default virtual `officina/lapis` remains auto: local-first + free overflow.
- `/profile auto|fast|deep|local`, persisted.
  - `auto`: local-first; free pool in default order.
  - `fast`: DeepSeek V4.1 Flash → MI​Mo Flash → other free fast lanes.
  - `deep`: free AgentRouter GPT/Claude → DeepSeek 4.1 Flash → MI​Mo.
  - `local`: no cloud.
- Status widget shows profile.

### Budget firewall
- Free-only invariant hard-coded for automation.
- `~/.vitriol/officina/budget.json`: daily/monthly token caps, per-provider
  counters; 80% warn, 100% refuse.
- Manual paid model selection requires explicit confirmation; never automatic.
- `/cost` shows allowance used/remaining and cache savings.

### Picker + status
- Model picker badges: `free`, `local`, `cooling`, `paid`.
- Route widget: `route cloud · deepseek-v4.1-flash · free · 2.1k tok`.

## Phase P2 — shared Caduceus proxy + subagents

### Proxy
- Local OpenAI-compatible proxy in front of VITRIOL 8279 + Caduceus 20128.
- Sticky sessions (`X-Session-Id`, fallback prefix hash) for cache affinity.
- Local-first; free cloud overflow; free-only guard.
- Shared ledger/health/budget from the TUI.
- User service, not boot-enabled by default.

### Subagents
- Subagents use the virtual default/shared router.
- Local when idle, free cloud when local busy.
- Read-only tool gating retained; concurrency capped.

## Phase P3 — key hygiene + polish
- Key ages, stale warnings, env backup/restore retention.
- Optional OS keyring (`secret-tool`) import/export; env file remains fallback.
- Stats dashboards, docs, long-run validation.

## Files
- `.pi/extensions/_shared/route-ledger.ts`, `provider-health.ts`,
  `profiles.ts`, `budget.ts`
- `.pi/extensions/resilient-default/*`, `adaptive-router/*`, `session-panel/*`
- `scripts/caduceus.mjs` (`stats`, `preflight`, key hygiene)
- `tui/src/tui/*` (commands/status/banner)
- `AGENTS.md`, `docs/OFFICINA.md`, `docs/officina-guide.md`

## Validation
- Unit: ledger rotation/redaction, cooldown persistence, profile ordering,
  budget guard, candidate filtering.
- Live: engine up/down, 429/404 recovery, restart persistence, proxy
  stickiness, subagent routing.
- TUI smoke, full JS/Rust tests, selfcheck.

## VITRIOL log reader (2026-11-10 16:18)
- New `_shared/vitriol-log.ts` tails `/tmp/opencode/vitriol_gen.log`
  (override: `VITRIOL_LOG_PATH`) and parses the newest task:
  prompt progress/tokens/prefill t/s, `n_gen`/`tg`/`tg_3s`, prompt/eval
  timings, draft acceptance/mean len, done/idle phase.
- `vitriol-decode` shows a second line from log truth:
  `◈ log task N · prefill 88% (2432 tok) · 362 t/s`
  or `◈ log task N · 284 gen · 28.3 t/s (3s 30.8) · accept 80%`.
- Falls back to the existing metrics line; log line only when fresh (<10s).
- Live verified with the running engine: log line reflected task 6295 done,
  26.4 t/s, 79% acceptance.

## Session-corruption prevention (2026-11-10 17:05)
- TUI now validates/quarantines the newest session before pi loads it:
  dangling tool calls, duplicate IDs, broken parent chains, broken compaction
  references, or synthetic recovery entries => `.corrupt-<ts>` + fresh session.
- Cross-process lock `~/.vitriol/officina/tui.lock`; `recover-session` refuses
  while the TUI is live. Stale PID detection allows offline recovery.
- `officina --new` forces a clean persisted session.
- Graceful quit (Ctrl+Esc / /quit) sends abort and waits briefly for pi to
  flush before killing the child.
- `vitriol caduceus session-doctor [--json]` reports session health.
- Root cause observed: long resumed session with 6 dangling calls + 4
  compactions + external recovery branches; pi turn_end boundary failed to
  resolve the assistant entry ID and stalled before tool execution.

## Fresh-session stall root cause + repair (2026-11-10 18:35)
- Fresh session had one dangling bash call because the turn was aborted
  before tool execution (likely Esc while the UI showed no progress). The
  old indicator ordering put vitriol-decode last, so a transient belowEditor
  widget could hide tk/s/log progress and invite an unnecessary abort.
- Fix 1: indicator order is now overflow-route -> vitriol-decode -> plan ->
  deep-research -> phase-model; the two decode/log rows always occupy the
  first three status slots.
- Fix 2: startup guard now **repairs** small sessions whose only defect is a
  dangling tool call (linearize active path + synthetic error results),
  preserving context. Complex sessions still quarantine and start fresh.
- Fix 3: graceful quit already aborts+waits; TUI lock blocks live recovery.
