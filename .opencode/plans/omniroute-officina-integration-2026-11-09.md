# OmniRoute ↔ Officina Integration Plan

- Date: 2026-11-09 12:17
- Status: implemented; live OpenCode Go round-trip completed via existing OpenCode auth
- Scope: `officina/`, `scripts/vitriol`, docs
- OmniRoute pin: `3.8.53` (latest npm at plan time)

## Objective

Make Officina a daily driver for subscription providers, starting with OpenCode Go,
by running OmniRoute as a local OpenAI-compatible gateway. Keep the VITRIOL engine
as the default provider so local cross-communication stays tightly coupled.

## Locked decisions

1. Integration shape: Officina extension + `vitriol omniroute` sync CLI.
2. Default: local VITRIOL engine. Explicit OmniRoute selection persists.
3. Catalog: only active OmniRoute connections. `/v1/models` already filters
   `isActive !== false`.
4. Passthrough: compression off, semantic cache off, memory off. Prompt cache
   stays on (cache breakpoints only, no content rewrite).
5. Install: pinned local vendor prefix at `~/.vitriol/vendor/omniroute`.
6. Cache disable: direct SQLite write while the gateway is stopped; per-request
   headers are the hard guarantee if the SQLite path fails.

## Architecture

- OmniRoute headless on `127.0.0.1:20128`.
- Officina registers provider `omniroute` at `http://127.0.0.1:20128/v1`.
- Canonical model ids: `provider/model` (e.g. `opencode-go/gpt-6-luna`).
- Provider headers on every request:
  - `X-OmniRoute-No-Cache: true`
  - `X-OmniRoute-Compression: off`
  - `x-omniroute-no-memory: true`
- `llama-cpp-provider` stops forcing `llamacpp` when the active provider is
  non-local.

## Environment names

Distinct from OmniRoute's own `OMNIROUTE_*` CLI vars to avoid auth/base-url
collisions:

- `OFFICINA_OMNIROUTE=1`
- `OFFICINA_OMNIROUTE_BASE_URL=http://127.0.0.1:20128/v1`
- `OFFICINA_OMNIROUTE_API_KEY=<local inference key>`

Stored in `~/.vitriol/officina/env`, mode 600.

## Phase 0 — bootstrap CLI

Files:

- `officina/scripts/omniroute.mjs`
- `scripts/vitriol` (`omniroute)` dispatch)

Commands:

- `vitriol omniroute setup [--opencode-key-stdin] [--no-install] [--force]`
- `vitriol omniroute up`
- `vitriol omniroute down`
- `vitriol omniroute status`
- `vitriol omniroute sync`

`setup` flow:

1. Install `omniroute@3.8.53` under the vendor prefix unless `--no-install`.
2. Start headless daemon on port `20128`; wait for `/api/health`.
3. Add OpenCode Go credential idempotently:
   `providers add opencode-go --credential-stdin --name opencode-go`.
4. Create a local inference API key via `POST /api/keys` with the local CLI token.
5. Stop gateway; write passthrough flags into `~/.omniroute/storage.sqlite`
   (compression API and cache-clear routes are dashboard-auth-only in 3.8.53;
   the CLI token is accepted by `/api/keys` but not those routes):
   - `databaseSettings` / `cache.semanticCacheEnabled` = `false`
   - `databaseSettings` / `cache.semanticCacheVectorEnabled` = `false`
   - `databaseSettings` / `cache.semanticCacheRequireZeroTemp` = `true`
   - `settings` / `memoryEnabled` = `false`
   - `compression` / `defaultMode` = `"off"`
   - `compression` / `enabled` = `false`
   - `compression` / `activeComboId` = `null`
   - leave `cache.promptCacheEnabled` untouched
6. Delete stale `semantic_cache` rows while stopped.
7. Restart gateway; refresh catalog cache.
8. Upsert `~/.vitriol/officina/env`; print status and next step.

## Phase 1 — Officina provider extension

Files:

- `officina/.pi/extensions/omniroute/index.ts`
- `officina/.pi/extensions/omniroute/catalog.ts`
- `officina/.pi/extensions/omniroute/catalog.test.ts`

Behavior:

- Skip when `OFFICINA_OMNIROUTE=0`, env missing, or gateway unreachable.
- Fetch `GET /v1/models?prefix=canonical` with bearer inference key.
- Map catalog fields to pi model entries:
  - `context_length` -> `contextWindow` (fallback 200000)
  - `max_output_tokens` / `maxOutputTokens` -> `maxTokens` (fallback 16384)
  - `capabilities.reasoning` -> `reasoning`
  - `capabilities.vision` / `attachment` / modalities -> image input
  - optional `pricing` -> cost rates
- Filter non-chat specialty rows and catalogs without chat/responses endpoints.
- Dedupe and sort.
- Seed from `~/.vitriol/officina/omniroute-models.json`; refresh live and write
  cache atomically.
- Register provider with strict-passthrough headers and `refreshModels(context)`
  so pi's model refresh re-fetches the live catalog.
- On failure: keep cache/local engine; never block Officina startup.

## Phase 2 — model sync gate

Files:

- `officina/.pi/extensions/llama-cpp-provider/index.ts`
- `officina/.pi/extensions/llama-cpp-provider/config.ts`
- `officina/.pi/extensions/llama-cpp-provider/sync-gate.test.ts`

Behavior:

- Pure `shouldArmEngineSync(provider?)`: true only for empty or `llamacpp`.
- `session_start`: skip forced local sync when active provider is non-local.
- `syncOnce`: if the provider changed to non-local, treat as aligned and stop
  the retry timer.
- Preserve existing retry and headless behavior for the local engine.

## Phase 3 — docs

Files:

- `docs/OFFICINA.md`
- `docs/officina-guide.md`

Document setup, status, sync, env vars, local-first guarantee, passthrough
headers, model selection persistence, troubleshooting, and rollback.

## Verification

- `cd officina && npx vitest --run .pi/extensions/omniroute .pi/extensions/llama-cpp-provider`
- `cd officina && npm run typecheck`
- `vitriol omniroute setup --opencode-key-stdin < /tmp/opencode.key`
- `vitriol omniroute status`
- `curl` model count and one cheap chat completion through the gateway.
- Officina `/model` shows OmniRoute models; selection survives restart.
- Switch back to local; selection survives restart.
- Confirm strict headers, cache bypass, no memory injection.
- Confirm env file mode 600 and key never printed.

## Risks / rollback

- Direct SQLite write couples to OmniRoute's `key_value` schema at 3.8.53.
  Non-fatal: per-request headers still enforce passthrough.
- Gateway down: Officina stays usable with cached models or local engine.
- Rollback: `vitriol omniroute down`, remove `OFFICINA_OMNIROUTE*` env lines,
  re-enable cache/compression in the dashboard. Local engine untouched.

## Out of scope

- Replacing pi native `/login` flows.
- Sidebar engine telemetry changes.
- Custom failover policy beyond OmniRoute's own fallback.
- Fixing the `llama-cpp-provider` config-path documentation discrepancy.

## Implementation notes (2026-11-09 12:36)

- OmniRoute `3.8.53` installed at
  `~/.vitriol/vendor/omniroute` successfully (1329 packages).
- `vitriol omniroute up|status|down` verified against the real gateway.
- `/api/keys` with the local CLI token returns 201 and a key; a temporary
  self-test key was created and deleted.
- Compression REST (`/api/settings/compression`) and cache-clear REST
  (`/api/cache/clear`) return 401/404 with the CLI token because those routes
  use `isAuthenticated`, which excludes the CLI token. Final implementation
  writes compression/cache flags directly to SQLite while the gateway is
  stopped and relies on per-request headers as the hard guarantee.
- Direct SQLite writes verified:
  `cache.semanticCacheEnabled=false`, `cache.semanticCacheVectorEnabled=false`,
  `compression.defaultMode="off"`, `compression.enabled=false`,
  `settings.memoryEnabled=false`.
- `npx vitest --run .pi/extensions/omniroute .pi/extensions/llama-cpp-provider scripts/omniroute-lib.test.mjs`
  -> 5 files, 70 tests passed.
- `npm test` -> 52 passed, 3 skipped files; 577 passed, 18 skipped tests.
- `npm run typecheck` -> clean.

## Live OpenCode check

Existing OpenCode auth was imported and a real
`opencode-go/glm-5.2` chat completion returned HTTP 200 through
`/v1/chat/completions` (short reasoning-only run; completion budget exhausted
by reasoning tokens). `zai` and `glm` coding-plan credentials are also
connected.

## Accounts automation (2026-11-09 13:34)

Scope approved by owner: API-key bulk import + assisted OAuth, config file with
env references only.

Catalog facts (OmniRoute 3.8.53, 375 providers):

- `api-key`: 252 (bulk importable)
- `oauth`: 28 total, 8 with CLI flows
- `web-cookie`: 35 (manual/dashboard)
- `local`: 14 (endpoint-dependent)
- `noauth`: 9 (explicit opt-in only)
- rest: search/audio/cloud-agent/upstream-proxy

Implemented additions:

- `~/.vitriol/officina/accounts.json` canonical config; `credential_env` names
  only, never literal secrets.
- `vitriol omniroute accounts init|sync|status|oauth`.
- `sync` builds one `providers import` payload, supports `--dry-run`, `--json`,
  `--only`, `--strict`; temp import file is 600 in a 700 dir and deleted in
  `finally`.
- `oauth list|login|status` wraps OmniRoute's 8 supported flows. Browser/device
  consent stays interactive; `cursor`/`zed` default to
  `--import-from-system`.
- Non-automatable: web-cookie, local, noauth-by-default, OAuth beyond CLI
  flows, account creation/billing.

### Accounts verification (2026-11-09 13:42)

- `vitriol omniroute accounts init --file /tmp/...` writes mode-600 JSON and
  refuses overwrite without `--force`.
- `accounts status` reads the 375-provider catalog and reports configured
  vs missing accounts; gateway-down path works.
- `accounts sync --dry-run` builds a provider import payload, skips missing
  env vars, and reports `created/skipped_existing/failed`.
- `accounts sync --dry-run --json` is pure JSON, includes no secret material
  (verified with a fake key; grep for the secret found nothing).
- `accounts oauth list` renders OmniRoute's 8 supported flows.
- `accounts oauth login` validates provider ids and rejects `kiro` without
  `--social`.
- Unit tests: `scripts/omniroute-lib.test.mjs` + `scripts/omniroute-accounts-lib.test.mjs`
  -> 24 passed.
- Full `npm test` -> 601 passed, 18 skipped (scripts tests now included); `npm run typecheck` clean.

## Free-plan automation (2026-11-09 15:00)

Implemented:

- `vitriol omniroute accounts free [--apply] [--json] [--strict]`
  - uses OmniRoute's built-in `/api/providers/free-onboarding` GET/POST
  - adds eligible zero-key no-auth providers
  - reuses OpenCode auth (`opencode-go`, `zai`, `zai-coding-plan`→`glm`)
  - installs Playwright `1.62.1` + Chromium when browser-backed free providers
    are added (`--no-browser` opts out)
- `vitriol omniroute accounts discover [--apply] [--json]`
  - standalone OpenCode auth reuse
- Catalog path switched from `?prefix=canonical` to `?prefix=alias`, then
  filtered by active OmniRoute connections. Canonical mode omitted no-auth
  aliases (`ddgw/`, `cfp/`, `unc/`).
- Pinned install now installs `omniroute@3.8.53 + playwright@1.62.1`; Chromium
  is installed separately by `accounts free --apply`.

Live results:

- Free onboarding created: `aihorde`, `cloudflare-playground`,
  `duckduckgo-web`, `uncloseai`.
- OpenCode auth imported: `opencode-go`, `zai`, `glm` (`glm-coding`).
- `uncloseai` removed: public beta closed, upstream returns 401.
- `naga-ac` removed: free models require an account key (upstream 401).
- `pollinations` not auto-added: OmniRoute server validation still requires a
  key despite registry `authType: optional`.
- `accounts free --plans` lists the 123 free-tier API-key providers with
  signup URLs and generated env-var names (no secrets, no signup automation).
- Remaining active connections: `aihorde`, `cloudflare-playground`,
  `duckduckgo-web`, `glm`, `opencode-go`, `zai` (6).
- DuckDuckGo chat smoke: OK.
- Cloudflare Playground chat smoke: OK after installing Playwright 1.62.1 +
  Chromium v1234. Error before install: HTTP 503 "needs Playwright and its
  Chromium browser".
- AI Horde reachable; rejects `max_tokens < 16` by design.
- Active-filtered alias catalog: 334 models (was 603 unfiltered; extra local
  CLI providers excluded).
- Setup created local inference key and wrote `~/.vitriol/officina/env`
  (mode 600); catalog shows 353 canonical / 603 alias models.

### Free path end-to-end (2026-11-09 15:02)

- Active-filtered alias catalog: 333 models across 6 connections.
- Officina one-shot through the registered provider:
  `vitriol officina -p --no-tools --provider omniroute --model ddgw/gpt-5.4-mini "Reply with exactly OK"`
  -> `OK`, exit 0. This also confirms `llama-cpp-provider` did not override the
  explicit OmniRoute selection (local engine was down).
- Cloudflare Playground chat smoke: OK after Playwright 1.62.1 + Chromium v1234.
- DuckDuckGo chat smoke: OK.
- AI Horde upstream reachable; enforces `max_tokens >= 16`.
- `npm test`: 607 passed, 18 skipped; `npm run typecheck` clean.
