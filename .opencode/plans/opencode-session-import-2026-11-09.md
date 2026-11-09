# OpenCode session import for Officina

- Date: 2026-11-09 18:27
- Status: implementing
- Entrypoint: `vitriol opencode`
- Scope: `officina/scripts`, `scripts/vitriol`, docs, selfcheck

## Findings

OpenCode sessions live in `~/.local/share/opencode/opencode.db` (SQLite,
currently 52 GB). The live schema has both legacy `message`/`part` rows and a
newer `session_message` table. The VITRIOL project has real sessions; the
current one has 881 messages / 3,640 parts and massive tool outputs.

pi (Officina) sessions are JSONL under
`~/.pi/agent/sessions/<encoded-cwd>/`. pi opens them with
`--session <path|id>` and forks with `--fork <path|id>`.

`opencode session list --format json` and `opencode export <id>` are available.

## Commands

```bash
vitriol opencode                         # TUI picker, current project
vitriol opencode list [--json]
vitriol opencode import latest [--no-launch]
vitriol opencode import <id> [--fork] [--full] [--dry-run]
vitriol opencode import <id> --last 80 --max-tool-chars 500
```

Defaults: current project, all messages (no arbitrary session-size cap), tool
input 200 chars, tool output 500 chars, reasoning 1k/message. `--last N` and
`--max-chars N` are explicit opt-in limits. `--full` raises content guards to
4k and includes everything OpenCode has under those guards.

## Implementation

Files:

- `officina/scripts/opencode-import-lib.mjs`
  - read-only `node:sqlite` loader (legacy + `session_message` fallback)
  - normalize OpenCode messages/parts
  - compact/full caps, block trimming
  - pi JSONL v3 writer
  - current-project filter, session list parser
- `officina/scripts/opencode-tui.mjs` (or embedded in opencode.mjs)
  - pi-tui session picker + action menu
- `officina/scripts/opencode.mjs`
  - `list`, `import`, TUI dispatch
- `officina/scripts/opencode-import-lib.test.mjs`
- `scripts/vitriol` `opencode)` dispatch (`--disable-warning=ExperimentalWarning`)
- docs + selfcheck

Mapping:

- user text -> pi user message
- assistant text -> text block
- reasoning -> thinking block (capped)
- tool part -> toolCall + bounded toolResult
- skip step-start/step-finish/snapshots
- preserve timestamps; model metadata recorded as historical
- write mode 600 atomically; no OpenCode writes

Destination:

`~/.pi/agent/sessions/--<cwd-dashed>--/<iso>_<uuid>.jsonl`

Launch via `vitriol officina --session <path>` or `--fork <path>` unless
`--no-launch`.

## Verification

- `vitriol opencode list --json` matches OpenCode’s list for this project.
- `vitriol opencode import latest --dry-run` prints counts/estimate only.
- Import a small session, open with `--session`, confirm history.
- `--fork` starts from the imported file.
- Tests: normalization, caps, ordering, JSONL header/parent chain, cwd filter.
- `npm test`, `npm run typecheck`, shell syntax.

## Implementation notes (2026-11-09 18:35)

- Added `officina/scripts/opencode-import-lib.mjs`, `opencode.mjs`, and tests.
- `vitriol opencode` TUI picker; `list` and `import` commands.
- Read-only SQLite loader supports legacy `message`/`part` and fallback
  `session_message`.
- Compact defaults now import **all messages**; arbitrary `last`/`max-chars`
  are opt-in. Per-tool output (500 chars) and reasoning (1k/message) guards
  remain because OpenCode stores multi-GB tool bodies.
- `--full` raises content guards to 4k; `--fork` uses pi `--fork`.
- Imported current-project latest session: 915 blocks, 1.9 MiB, 1,842 pi
  messages, written under `~/.pi/agent/sessions/...`.
- Small 8-block import opened successfully with
  `vitriol officina --session <path>`.
- TUI PTY smoke passed.
- `npm test`: 645 passed, 18 skipped; typecheck clean.

Note: a DuckDuckGo free model rejected one imported-tool-history request with
HTTP 400; local/OpenCode models accepted the same session. This is a
provider-specific tool-schema limitation, not an import failure.
