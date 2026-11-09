# Officina settings TUI

- Date: 2026-11-09 18:16
- Status: implemented
- Entrypoint: `vitriol settings`
- Implementation: `officina/scripts/settings-tui.mjs`

## Goal

A searchable full-screen menu for every VITRIOL engine setting and every
Officina env setting, with explanations and safe editing.

## Behavior

- Reads `~/.vitriol/config` (INI) and `~/.vitriol/officina/env`.
- Engine keys: inline comment blocks become explanations; curated enums
  (`on/off`, `true/false`, cache/KV/spec modes) cycle; free-form values open a
  one-line editor.
- Officina env: curated explanations for known OFFICINA_* keys, falling back
  to inline env comment blocks.
- Secret env keys (`*_API_KEY`, `*_TOKEN`, `*_SECRET`, `*_PASSWORD`,
  `*_CREDENTIAL`) are masked and not editable in the TUI.
- Engine edits: atomic write, one-time `.bak` backup, preserve file mode.
- Env edits: atomic write through `upsertEnvFile`, mode 600.
- Engine changes require `systemctl restart vitriol-server`; env changes apply
  to the next Officina session.
- Search, arrows, Enter/Space edit/cycle, Esc quit, Ctrl+C quit.
- Non-interactive inspection: `vitriol settings --list` / `--json`.
- Overrides for testing: `VITRIOL_CONFIG`, `OFFICINA_ENV_FILE`.

## UI

- `@earendil-works/pi-tui` `SettingsList` with alternate main-screen renderer.
- OKLCH accent colors, bold selected labels, colored values, dim descriptions.
- Header states the restart requirement; footer status shows the last save.

## Verification

- `settings --json` enumerates 63 settings.
- Secret `OFFICINA_OMNIROUTE_API_KEY` renders `•••• (set)`; no key leakage.
- `npm test`: 638 passed, 18 skipped.
- `npm run typecheck`: clean.
- PTY smoke via `script` rendered the TUI without startup errors.
