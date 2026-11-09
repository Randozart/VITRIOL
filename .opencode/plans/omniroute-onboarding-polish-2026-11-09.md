# OmniRoute onboarding polish

- Date: 2026-11-09 15:50
- Status: implementing
- Scope: `officina/scripts`, `scripts/vitriol`, docs
- Predecessor: `.opencode/plans/omniroute-officina-integration-2026-11-09.md`,
  `.opencode/plans/officina-cloud-cache-praetor-egress-2026-11-09.md`

## Goal

Make a fresh OmniRoute + Officina setup one command, diagnosable, boot-persistent,
and easy to extend with free API-key providers.

## 1. `vitriol omniroute setup --free`

Current setup: install, add OpenCode provider, create inference key, disable
cache/compression, write env, refresh catalog.

Add `--free`:
- after normal setup, run free onboarding (`accounts free --apply` logic):
  - add eligible zero-key providers;
  - reuse existing OpenCode auth;
  - install Chromium when browser-backed providers are included;
- enable provider cache overrides for `opencode-go` and `glm` when present;
- leave `--no-browser` passthrough for air-gapped/slow installs.

One command on a clean box:

```bash
vitriol omniroute setup --free --opencode-key-stdin < keyfile
```

## 2. `vitriol omniroute doctor`

Read-only health report. Checks:

- omniroute binary + pinned version
- gateway health
- inference key present + env file mode
- active connections (count/providers)
- active catalog model count
- cache overrides on `opencode-go`/`glm`
- Playwright + Chromium runtime
- Praetor binary + `OFFICINA_PRAETOR`
- egress mode (`off` is a warning, not failure)
- systemd user service installed/active
- accounts config present (`accounts.json`)

Output: PASS/WARN/FAIL + `--json`, exit 1 only on hard failures.

## 3. `vitriol omniroute service install|status|remove`

Systemd **user** unit: `~/.config/systemd/user/vitriol-omniroute.service`.

- `ExecStart=%h/.vitriol/vendor/omniroute/node_modules/.bin/omniroute serve --headless --port 20128`
- `Environment=PORT=20128`, `OMNIROUTE_HEADLESS=1`
- `EnvironmentFile=-%h/.vitriol/officina/egress.env`
- `Restart=on-failure`, `RestartSec=2`
- `WantedBy=default.target`

Behavior:
- `install`: write unit (644), regenerate `egress.env` from `egress.json`,
  stop the CLI daemon to free the port, `systemctl --user daemon-reload`,
  `enable --now`, best-effort `loginctl enable-linger`.
- `status`: installed/active/enabled + gateway health.
- `remove`: `disable --now`, remove unit, daemon-reload.

Egress `set/clear` also rewrites `egress.env` so the unit sees proxy changes.

## 4. `vitriol omniroute accounts free --template`

Generate `~/.vitriol/officina/accounts.json` with all free-tier API-key
providers:

- `provider`, `name`, `credential_env`, `website`, `enabled: false`
- `oauth` list of free OAuth providers for assisted login
- no secrets; 600 file
- refuses overwrite without `--force`

This turns the 123-provider signup list into a fill-in file; `accounts sync`
only imports entries whose `enabled` is true and env var is set.

## Verification

- unit tests: free template builder, systemd unit builder, existing cache mode.
- `vitriol omniroute setup --free` idempotent on the current box.
- `vitriol omniroute doctor` on current box: gateway/key/connections/catalog/
  cache/browser pass; egress/service/accounts warnings expected.
- `vitriol omniroute service install` then `status`; remove restores manual mode.
- `accounts free --template --file /tmp/...` writes 123 disabled entries.
- full `npm test` + `npm run typecheck`.

## Implementation notes (2026-11-09 16:00)

Implemented:

- `setup --free`: normal setup then free onboarding, existing-auth reuse, and
  cache enable for `opencode-go`/`glm`. Verified idempotent on the current box
  (0 candidates, 0 imports, cache overrides kept).
- `doctor`: 12 checks, human + `--json`, warning-only for optional egress /
  accounts config; exit 1 only on hard failures.
- `service install|status|remove`: systemd user unit with
  `EnvironmentFile=-%h/.vitriol/officina/egress.env`; linger enabled
  (`Linger=yes`); install/remove/reinstall verified. Gateway is now managed by
  `vitriol-omniroute.service`.
- `accounts free --template`: writes 123 disabled free API-key entries with
  provider/name/credential_env/website and the 6 free OAuth providers. Refuses
  overwrite without `--force`.
- `startServer`/`stopServer` prefer the user service when the unit is installed,
  so egress changes restart the systemd service rather than spawning a stray CLI
  daemon.
- Egress `set/clear` rewrites `egress.env` for systemd and restarts the service.

Live verification:

- `vitriol omniroute doctor` -> 0 failures, 2 warnings (egress off, accounts
  config not created).
- `vitriol omniroute service status` -> installed + enabled + active.
- `systemctl --user status vitriol-omniroute.service` -> active; `Linger=yes`.
- `egress set` with dummy proxy and `egress clear` both restarted the service
  and recovered gateway health.
- `accounts free --template --file /tmp/...` wrote 123 disabled entries; sync
  reported `SKIP 123 disabled entry(s)` and imported nothing.
- `setup --no-install --free` against the running service stayed active and
  idempotent.
