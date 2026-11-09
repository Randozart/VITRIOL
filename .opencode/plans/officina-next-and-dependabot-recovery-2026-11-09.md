# Officina next steps + Dependabot recovery

- Date: 2026-11-09 16:27
- Status: implementing
- Scope: Dependabot PR branches, `officina/`, docs

## Part A — Dependabot recovery (done)

Updated/repaired branches:

- #5 `vitriol-governor`: migrate ureq 2 → 3 (`Agent::config_builder`,
  `body_mut().read_to_string`, `send`/`call`). Tests pass.
- #6 `libhermes`: sha2 0.11 `finalize()` no longer implements `LowerHex`;
  encode bytes explicitly. Tests pass.
- #7 `libvitriol`: refresh `vitriol-tui/Cargo.lock` because it consumes
  libvitriol by path. `cargo test --locked` passes.
- #8 `vitriol-tui`: migrate poller/search/officina HTTP calls to ureq 3.
  `cargo test --locked` passes (158 tests).
- #11 `officina` npm: adapt to pi 1.5 discriminated `before_agent_start`
  event; normalize `ProviderHeaders` (`string | null`) before compaction.
  Typecheck + 562 tests pass.
- #12/#13/#14 rustls: branches rebased/updated; waiting on CI.

Ready-but-unmerged: #3, #4, #9, #10.

## Part B — Officina next steps

### 1. `vitriol omniroute use <model>`

Persist the default pi model/provider so a new session opens directly on
OmniRoute without `/model`:

- validate model id against the active catalog
- atomically update `~/.pi/agent/settings.json`
  (`defaultProvider: "omniroute"`, `defaultModel: <id>`)
- `use --local` resets to `llamacpp` / existing local model
- print the selected model and restart hint

### 2. `cache-report --cwd`

Scope the report to a project session directory using pi's encoded cwd
(`~/.pi/agent/sessions/--path-with-dashes--`) instead of the global latest
session.

### 3. Remaining manual steps

- `accounts free --template` → fill env vars for additional free API-key
  providers → `accounts sync`.
- Generic free OAuth providers (`openference`, `qoder`, `agy`, `freebuff`) still
  need dashboard/consent; CLI covers only OmniRoute's 8 native flows.
- Fixed egress needs a real proxy/WireGuard endpoint.
- Praetor is opt-in: `OFFICINA_PRAETOR=1`.

## Verification

- `npm test` / `npm run typecheck` on pi 1.5 branch.
- `vitriol omniroute use <id>` then inspect `~/.pi/agent/settings.json` and
  start a one-shot session.
- `vitriol cache-report --cwd /path/to/project`.
- Re-check Dependabot PR checks after pushes.

## Implementation notes (2026-11-09 16:35)

- `vitriol omniroute use <model>` implemented and verified:
  - `use ddgw/gpt-5.4-mini` set `defaultProvider=omniroute`,
    `defaultModel=ddgw/gpt-5.4-mini`.
  - `use --local "Lapis Occultus"` restored the local default.
  - Settings write is atomic and mode 600.
- `vitriol cache-report --cwd DIR` implemented; verified against the VITRIOL
  project session directory.
- Dependabot fixes pushed to branches:
  - #5 ureq 3 migration
  - #6 sha2 hex encoding
  - #7 vitriol-tui lockfile refresh
  - #8 vitriol-tui ureq 3 migration
  - #11 pi 1.5 event/header compatibility
- #6, #5, #7, #8, #11 local checks pass (Rust tests / npm tests + typecheck).
