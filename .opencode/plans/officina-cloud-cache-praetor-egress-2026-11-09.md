# Officina: cloud cache mode, native Praetor, fixed egress

- Date: 2026-11-09 15:23
- Status: implementing
- Scope: `officina/.pi/extensions`, `officina/scripts`, `scripts/vitriol`, docs
- Related: `.opencode/plans/omniroute-officina-integration-2026-11-09.md`

## Objective

Keep Officina's local-engine low-context behavior as the default, but when a
cloud provider is selected:

1. optimise for provider prompt-cache hits;
2. expose Praetor as a first-class, cache-safe verification utility;
3. support a fixed proxy/VPN egress for privacy/geo/latency.

Explicit non-goal: automatic IP rotation to multiply free-tier quotas. That is
quota evasion/anti-abuse territory and is not implemented.

## Decisions (owner-approved)

- Cache mode: `OFFICINA_CACHE_MODE=auto` — local engine keeps current behavior;
  non-local providers switch to cache-safe mode.
- Praetor: advisory + report. Inject instructions, post-edit validate, append
  diagnostics tail, expose `praetor_report`. Never block edits.
- Egress: fixed HTTP/SOCKS5 or OS-level WireGuard only. No auto-rotation.

## 1. Cloud cache mode

### Model

- `auto`: cloud iff active provider is not `llamacpp`.
- `local`: local-style everywhere.
- `cloud`: cache-safe even on local (explicit override).
- `off`: disable cache-mode behavior.
- Env override: `OFFICINA_CACHE_MODE=auto|local|cloud|off`.
- Cross-extension state uses a `globalThis` singleton (`_shared/cache-mode.ts`)
  because jiti isolates module state between extensions.

### Cache-safe rules

- Stable prefix: system prompt + tool schemas + earliest history stay
  byte-identical across turns.
- Dynamic injections are tail-only. Existing task-state, scratchpad,
  diagnostics-loop, memory, route notices, relay cards already append.
- `tool-result-clearer` is the main prefix mutator. In cloud mode it only runs
  when context usage is at/above `OFFICINA_CACHE_CLEAR_PCT` (default 92) or
  `OFFICINA_CACHE_FORCE_CLEAR=1`.
- Pi compaction remains enabled; compaction is an explicit cache reset.
- Model/provider switch is an explicit cache epoch reset.

### Provider-native caching

- OmniRoute already injects `prompt_cache_key` for known caching providers
  (OpenAI/Codex/Azure, zai, DeepSeek, Kimi) and Claude-format providers get
  heuristic `cache_control` breakpoints.
- Add connection override for `opencode-go` and `glm`:
  `providerSpecificData.cache.supportsPromptCaching = true`.
- A/B each provider; revert a connection if upstream ignores/rejects the hint.
- Add pi model compat for OmniRoute models:
  `sendSessionAffinityHeaders: true`, `sessionAffinityFormat: "openai"`.
- Do not force `cacheControlFormat: "anthropic"` globally; only consider for
  known Claude-format aliases after smoke test.

### Telemetry

- Session panel: `cacheRead`, `cacheWrite`, hit-rate row (priority 21).
- Keep pi cache-miss notices available; cloud mode surfaces them.
- `officina/scripts/cache-report.mjs`: parse pi session JSONL, report turns,
  input/output/cache tokens, hit rate, missed tokens/cost estimate, idle gaps,
  model switches. Accept `--file`/`--json`.

### Config

```yaml
coding:
  cache:
    mode: auto
    native_provider: true
    stable_prefix: true
    clear_only_above_pct: 92
    telemetry: true
    reset_epoch_on_model_change: true
```

Env:
- `OFFICINA_CACHE_MODE`
- `OFFICINA_CACHE_CLEAR_PCT`
- `OFFICINA_CACHE_FORCE_CLEAR=1`

## 2. Native Praetor

Praetor binary: `~/.cargo/bin/praetor` (0.1.0). Existing integration is env-only
`TRIS_DIAG_PRAETOR=1` and single-file temp validation in diagnostics-loop.

### Behavior

- Config toggle (env first, unified-config key added to validator):
  `OFFICINA_PRAETOR=1` / alias `TRIS_DIAG_PRAETOR=1`.
- Kill switch: `OFFICINA_NO_PRAETOR=1`.
- Missing binary → clean disable with one notice.
- `session_start`: run `praetor instruct` once; inject as a tail/system block.
  Block is static per binary version.
- On edit/write `tool_result`: collect changed file.
- On next `context`: copy file to temp dir, run
  `praetor validate --warn --json --target <tmpdir>`, parse failures, append a
  bounded diagnostics tail (`OFFICINA_PRAETOR_BUDGET`, default 300 chars).
  Directory target is required — a file target silently passes.
- `praetor_report` tool: run `praetor report --target <cwd> --format markdown`;
  cap output and summarise.
- Advisory only; no edit blocking.
- Keep `TRIS_DIAG_PRAETOR` compatibility alias; remove the duplicate branch from
  diagnostics-loop once migrated.

### Cache interaction

- Instructions injected once; no per-turn system-prompt rewrite.
- Diagnostics are append-only tails; no prefix invalidation.

## 3. Fixed egress

### Commands

```bash
vitriol omniroute egress status
vitriol omniroute egress set --proxy socks5://127.0.0.1:1080
vitriol omniroute egress set --proxy http://user:pass@host:8080 --no-proxy 127.0.0.1,localhost
vitriol omniroute egress clear
vitriol omniroute egress test
```

### Config

`~/.vitriol/officina/egress.json` (mode 600):

```json
{
  "mode": "fixed",
  "proxy": "socks5://127.0.0.1:1080",
  "no_proxy": ["127.0.0.1", "localhost", "::1"]
}
```

- `startServer` merges `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` (upper+lower) and
  `NO_PROXY` into the OmniRoute daemon environment.
- OmniRoute's `open-sse/utils/networkProxy.ts` already consumes these env vars.
- `egress set/clear` restarts the daemon when it is running.
- `egress status` masks proxy credentials.
- `egress test` uses `curl --proxy` to `https://api.ipify.org?format=json` and
  reports gateway health.
- Kill switch: `OFFICINA_EGRESS=0`.
- OS-level WireGuard documented separately; no root automation.

Explicit non-goal: proxy rotation on 429/quota/IP-gate.

## 4. Implementation order

1. Plan doc + cache-mode shared state + tests.
2. Cache-policy extension, tool-result-clearer gating, session-panel row.
3. Cache-report script.
4. Provider cache override command (`vitriol omniroute cache enable|status|disable`)
   + live A/B.
5. Praetor extension + tests; migrate diagnostics-loop branch.
6. Fixed egress commands.
7. Docs, selfcheck, full tests.

## 5. Verification

Cache:
- 3-turn fixed prompt, `auto` vs `off`; turn-2 `cacheRead > 0`.
- `cache-report` hit rate/missed cost.
- Tool-result-clearer does not mutate prefix below 92% in cloud mode.
- Provider overrides tested individually; bad ones reverted.

Praetor:
- Missing binary disables cleanly.
- Instruct injected once; system prompt stable.
- Post-edit diagnostics only on failures.
- `praetor_report` capped.
- `npx vitest` + `npm run typecheck`.

Egress:
- `status/set/clear/test`.
- Daemon sees proxy env; provider request succeeds.
- Localhost bypasses.
- No rotation code.

## 6. Risks / rollback

- Cache hints can be ignored or rejected by some OpenAI-compatible upstreams;
  per-connection rollback with `cache disable`.
- Cache mode disabling clearing can raise context use; threshold/force-clear
  escape hatch.
- Praetor validation can be slow; 60s timeout and advisory mode.
- Fixed egress can fail closed or leak if `NO_PROXY` is wrong; status/test and
  explicit clear.
- No secrets printed or written to config; proxy credentials stay in egress file
  (600) and daemon env.

## Implementation notes (2026-11-09 15:33)

Implemented:

- `_shared/cache-mode.ts` + tests + jiti-isolation contract.
- `cache-policy` extension sets mode on `session_start`/`model_select`.
- `tool-result-clearer` defers prefix mutation below the cloud threshold.
- Session-panel cache row (`cacheRead/cacheWrite/hit%`).
- `officina/scripts/cache-report.mjs` + lib/tests; `vitriol cache-report`.
- `vitriol omniroute cache status|enable|disable`.
- `officina/.pi/extensions/praetor/` advisory extension + tests.
- `diagnostics-loop` no longer duplicates Praetor when `OFFICINA_PRAETOR=1`.
- `vitriol omniroute egress status|set|clear|test` (fixed only).
- Docs: `docs/OFFICINA.md`, `docs/officina-guide.md`, `scripts/vitriol`.

Live cache A/B (enabled):

- `opencode-go/glm-5.2`, identical 4024-token request twice:
  - turn 1: `cached_tokens=0`
  - turn 2: `cached_tokens=3840`
- `cache enable --providers opencode-go,glm` persisted on the connections.
- `zai` left without the override (OmniRoute already recognises it as a
  caching provider).

Verification:

- `npm test`: 629 passed, 18 skipped (60 files).
- `npm run typecheck`: clean.
- `OFFICINA_CACHE_MODE=auto OFFICINA_PRAETOR=1` one-shot through
  `omniroute/ddgw/gpt-5.4-mini` returned OK (extensions load cleanly).
- `cache-report` parsed a real session JSONL.
- `egress status` works; invalid scheme rejected without writing config.
- Praetor `validate --warn --json --target <tmpdir>` smoke: `{"failures":[],"passed":true}`.

### Final verification addendum

- `vitriol omniroute cache status` shows `opencode-go` and `glm` enabled,
  `zai` left to OmniRoute's built-in caching policy.
- `vitriol omniroute egress status` shows `mode=off`, gateway up, fixed-only
  policy; invalid `ftp://` proxy rejected without writing config.
- `npm test` after all unit additions: 629 passed, 18 skipped.
- `npm run typecheck`: clean.
- Fixed egress `set`/`status`/`clear` exercised with a dummy proxy; daemon
  restarted cleanly and recovered after clear. `egress test` still needs a real
  proxy/WireGuard endpoint.
