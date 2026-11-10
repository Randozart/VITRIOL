# Caduceus cutover + resilient default (2026-11-10 14:16)

Status: COMPLETE 2026-11-10 14:33.

Shipped:
- Caduceus rename across scripts, extension, provider id, env keys, cache, CLI,
  service unit; hard cut (old `vitriol omniroute` gone).
- Migration applied: env/settings/sessions/cache/unit; backup at
  `~/.vitriol/officina/migrate-backup-2026-11-10T13-24-55-842Z`.
- `vitriol-caduceus.service` active; old user unit removed.
- Virtual `officina/lapis` ("Lapis Occultus") registered; new-session default.
- Resilient route live-verified: engine down -> Caduceus
  `opencode-go/deepseek-v4.1-flash`; model picker lists `officina/lapis`
  with VITRIOL unreachable.
- Attribution added to `NOTICE` + `officina/THIRD_PARTY_NOTICES.md`.
- Chromium installed; Cloudflare Playground free lanes respond (tool calling is
  emulated by the vendor).
- Tests: 689 JS, 98 Rust, `tsc` clean, selfcheck PASS.

## Goals
1. Drop the OmniRoute name from Officina-facing surfaces; rename to **Caduceus**.
2. Keep full MIT attribution to OmniRoute in NOTICE/provenance; vendor internals
   keep their upstream names.
3. Add an always-selectable virtual default model **Lapis Occultus**
   (`officina/lapis`) that routes local-first and fails over to free/cloud.

## Hard-cut migration
- Provider id: `omniroute` -> `caduceus`; model ids `caduceus/...`.
- CLI: `vitriol omniroute` -> `vitriol caduceus` (old command removed).
- Scripts: `omniroute*.mjs` -> `caduceus*.mjs`; update imports, selfcheck, settings-tui.
- Extension: `.pi/extensions/omniroute` -> `.pi/extensions/caduceus`.
- Env: `OFFICINA_OMNIROUTE[_BASE_URL|_API_KEY]` -> `OFFICINA_CADUCEUS...`.
- Cache: `omniroute-models.json` -> `caduceus-models.json`.
- Service: `vitriol-omniroute.service` -> `vitriol-caduceus.service`.
- `vitriol caduceus migrate --dry-run|--apply`:
  - backup env/settings/sessions/cache/unit;
  - rewrite env keys, cache filename, pi settings, session JSONL provider/model ids;
  - stop/disable old unit, install/start new unit, verify health;
  - delete old names after success.
- No runtime alias/fallback: missed references fail visibly.

## Attribution (must stay)
- `NOTICE`: add OmniRoute MIT notice + upstream link.
- `officina/THIRD_PARTY_NOTICES.md`: vendored package, MIT license reference.
- `vitriol caduceus status` prints `Caduceus · powered by OmniRoute <ver> (MIT)`.
- Vendor path `~/.vitriol/vendor/omniroute`, package `omniroute`, data dir
  `~/.omniroute`, `OMNIROUTE_HEADLESS`, `X-OmniRoute-*` headers stay.

## Resilient virtual default
- New `.pi/extensions/resilient-default`.
- `pi.registerVirtualModel({ provider: "officina", id: "lapis",
  name: "Lapis Occultus", ... })`.
- Keyless provider -> always selectable, even with VITRIOL down.
- Route:
  - user: local if up/idle; free/cloud overflow if busy; cloud failover if down.
  - continuation: previous physical model (tool-loop cache).
  - retry: local failure -> cloud; cloud failure -> next candidate/local.
  - direct: local if up else cloud.
  - sensitive/confidential: local only.
- Free candidates: Caduceus catalog discovery, cooldowns, per-owner limits.
- New-session default: `officina/lapis`.
- `llama-cpp-provider` must not override a virtual selection at startup.
- `adaptive-router` skips when the selected model is virtual.
- Session-panel treats virtual as routed, not a mismatch.
- Compaction override for `officina/lapis`.

## Verification
- `vitriol caduceus doctor`; gateway unit active; no old unit.
- `grep -R` for `omniroute|OmniRoute` shows only vendor/attribution/history.
- Model picker shows `Lapis Occultus` with VITRIOL stopped.
- Engine-down turn answers from cloud; engine restart returns local.
- Sensitive prompt stays local.
- Full JS tests + Rust tests; build/install TUI; restart gateway + TUI.

## Free-tier signup wizard (2026-11-10 14:40)
- `vitriol caduceus signup` (and `accounts signup`): guided manual signup over
  all 123 free-tier API-key providers, one at a time.
- Opens the official signup URL; prints env var + free-tier note.
- Keys: direct paste at the action prompt or `k` (hidden input). Written
  atomically to `~/.vitriol/officina/env` mode 600; accounts.json entry enabled;
  provider synced immediately unless `--no-sync`.
- Progress `~/.vitriol/officina/signup-progress.json`; resume/skip/redo/reset.
  Enter=next, s=skip, q=quit, o=reopen.
- `accounts.json` seeded with 123 disabled entries.
- No automated signup, no multi-account farming; one account per provider.

## Key visibility + env-file status (2026-11-10 14:52)
- `vitriol caduceus keys [--all] [--json]`: lists provider keys that are
  PRESENT across `~/.vitriol/officina/env` and the shell, with last-4 masking,
  source (`env-file`/`shell`), and gateway link status.
- `vitriol caduceus accounts status` now merges the env file into its lookup,
  so it no longer reports keys as missing when only the file has them.
  It prints `Keys present: N/M` and `gateway connections matched: N`.
- `accounts sync` / signup sync also use the merged env, so shell invocations
  work without manually exporting keys.
- Verified: `keys` shows agentrouter + agnes present/env-file/gateway;
  `accounts sync --dry-run` plans 2 imports, 0 errors.

## Bulk key enable/import (2026-11-10 15:03)
- `vitriol caduceus accounts enable [--sync] [--disable-missing] [--dry-run]`:
  enables every account whose `credential_env` is present in the env file or
  shell; optional immediate sync. Reports present/missing counts.
- `vitriol caduceus accounts import-keys <keys-file>`: reads a `.env`-style
  bundle, backs up the env file, writes keys atomically, enables matching
  accounts, and syncs (unless `--no-sync`). Supports `--dry-run` and `--json`;
  never prints key values.
- `vitriol caduceus keys [--all]`: masked present-key listing with source and
  gateway-link status.
- Verified: dry-run matched GROQ, ignored an unknown key; actual import wrote
  and enabled, then test state was restored.

## Dahl Inference (2026-11-10 15:18)
- The existing `DAHL_API_KEY` is gateway-linked and verified
  (`dahl/MiniMaxAI/MiniMax-M2.7`, 204k ctx, tool calls, 200 OK).
- Added `dahl` to the free-candidate owner order after `opencode-go`,
  before `glm`/`zai`/`cfp`; unknown pricing is accepted for Dahl because the
  provider offers a free token allowance.
- 404/410 model responses now cool that model for 1h (`Kimi-K2.6` is listed
  by the catalog but currently not offered upstream).
- One key/account only. No farming/multi-account automation was added.

## Free coverage + model preference (2026-11-10 15:39)
- 15 signed-up provider keys, all gateway-linked.
- 48 free tool-capable overflow candidates (48-cap) across: opencode-go 8,
  no-think 3, dahl 2, agentrouter 3, agnes 3, api-airforce 7, aion 5, cohere 6,
  bytez 1, glm 1, cfp 8, freemodel-dev 1.
- Candidate preference: DeepSeek V4.1 Flash first, other DeepSeek Flash lanes,
  then MiMo Flash, then GLM Flash, then other free tool models; owner health
  still sorts before model family.
- Rationale: GLM 5.3 Flash is paid on the `glm` provider; only `glm-4.7-flash`
  is free (quota exhausted). MiMo Flash is zero-cost on opencode-go and 1M ctx.
- Linked keys with no current free tool candidates: electronhub, free-ai,
  auriko, freetheai (catalog exposure gap); baseten excluded (paid credits).
