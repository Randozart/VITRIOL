# Officina: free OAuth flows + env config toggles

- Date: 2026-11-09 17:53
- Status: implementing
- Scope: `officina/scripts`, `scripts/vitriol`, docs, selfcheck

## Goal

Finish the remaining Officina/OmniRoute work:

1. Generic free OAuth flows not covered by OmniRoute's native CLI list.
2. Env-file config toggles for cache mode and Praetor.
3. Docs/tests/selfcheck.

## Phase 1 — Free OAuth

Providers to add:

| id | flow | backend | notes |
|---|---|---|---|
| `openference` | browser PKCE | `openference` | paste callback URL/code |
| `agy` | browser PKCE | `agy` | Antigravity CLI identity |
| `qoder` | browser auth-code | `qoder` | disabled unless `QODER_OAUTH_*` env is set |
| `freebuff` | device code | `freebuff` | code + URL, poll |
| `amazon-q` | social alias | `kiro` | delegate to Kiro |

Native flows stay on `omniroute providers auth`:
`gemini`, `antigravity`, `cursor`, `zed`, `kiro`, `claude-code`, `codex`,
`copilot`.

New files:
- `officina/scripts/omniroute-oauth-lib.mjs`
  - provider/flow/backend map
  - `resolveBackendKey`
  - `parseOAuthCallback(input, expectedState)`
- `officina/scripts/omniroute-oauth-lib.test.mjs`

Implementation:
- `accounts oauth list` prints the unified native + free table.
- `accounts oauth login <id>`
  - native ids -> existing `providers auth` path
  - free browser ids -> GET `/api/oauth/<key>/authorize`, print/open URL,
    prompt callback, POST `/exchange`
  - `freebuff` -> GET `/api/oauth/freebuff/device-code`, print code/URL,
    POST `/poll` with slow_down handling
  - `amazon-q` -> Kiro social flow
- Best-effort browser open via `xdg-open`; `--no-browser` disables.
- No cookie scraping, no account creation.

## Phase 2 — Config toggles

Commands:

```bash
vitriol omniroute config get [--json]
vitriol omniroute config set cache auto|local|cloud|off
vitriol omniroute config set praetor on|off
```

Mapping:

- `cache` -> `OFFICINA_CACHE_MODE`
- `praetor` -> `OFFICINA_PRAETOR` (`on` -> `1`, `off` -> `0`)

Writes through the existing atomic env upsert; mode 600; no gateway restart.

## Phase 3 — Verification

- `npx vitest --run scripts/omniroute-oauth-lib.test.mjs`
- `npm test`, `npm run typecheck`
- `vitriol omniroute accounts oauth list`
- `vitriol omniroute config get`
- set `cache auto` / `praetor on`, inspect env, restore if needed
- no live OAuth login without user consent; validate authorize/poll paths only.

## Non-goals

- Automating account creation/CAPTCHA.
- Cookie scraping for web-cookie providers.
- Qoder flow without its required `QODER_OAUTH_*` server env.

## Implementation notes (2026-11-09 17:58)

- Added `officina/scripts/omniroute-oauth-lib.mjs` with the unified native+free
  provider map, backend aliasing, and callback parsing.
- `accounts oauth list` now prints native and free flows:
  `openference`, `agy`, `qoder`, `freebuff`, plus `amazon-q` (kiro alias).
- `accounts oauth login`:
  - native ids keep the OmniRoute `providers auth` path;
  - free browser flows use `/api/oauth/<key>/authorize` + `/exchange`;
  - `freebuff` uses `/device-code` + `/poll`;
  - `amazon-q` delegates to `kiro --social`.
- Added `vitriol omniroute config get|set` for
  `OFFICINA_CACHE_MODE` and `OFFICINA_PRAETOR` in `~/.vitriol/officina/env`.
- Docs, scripts help, and selfcheck updated.

Verification:
- `omniroute-oauth-lib.test.mjs`: 7 passed.
- `npm test`: 638 passed, 18 skipped (61 files).
- `npm run typecheck`: clean.
- `accounts oauth list` renders all 13 flows.
- `config get` shows cache=auto, praetor=off, egress=off, omniroute=1.
- `config set praetor on` wrote `OFFICINA_PRAETOR=1`; restored to off.
- No live OAuth login was initiated (needs user consent).
