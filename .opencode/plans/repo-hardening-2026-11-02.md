# Repo hardening — CI, locks, metadata (2026-11-02)

## Scope

Track A from the repo audit, plus the two owner choices: per-crate
`Cargo.lock` tracking and a one-time `cargo fmt` pass. Work landed directly on
`main` (owner decision); branch protection was enabled only after CI was green.

## Commits

```
977eafd ci: auto-rebase dependabot PRs
8fe876c ci: fix warning flags, cache workspaces, and python lint blockers
9370ea4 chore: add repo metadata and community files
978c51b ci: add GitHub Actions, dependabot, and shellcheck fix
c91a8d1 fix(officina): track package-lock and add jiti for typecheck
87ab9c3 style: rustfmt pass across crates
5bfd9af chore(deps): track Cargo.lock per crate
```

## Changes

### Dependencies / reproducibility

- Removed `Cargo.lock` from `.gitignore`; committed all 10 crate locks.
- Added `rust-toolchain.toml` pinned to `1.94.1` (rustfmt + clippy).
- Un-ignored and committed `officina/package-lock.json`; added `jiti@2.7.0`
  as an explicit dev dependency because `tsc` could not resolve the
  transitive copy nested under `@earendil-works/pi-coding-agent`.
- Verified `cargo check/test --locked` for the CI matrix before pushing.

### Formatting

- `cargo fmt` across 10 crates: 47 files, +1688/−810 (mechanical).
- `cargo fmt --check` now passes everywhere.

### CI (`.github/workflows/ci.yml`)

- Rust matrix: `libvitriol`, `vitriol-cache`, `vitriol-governor`,
  `vitriol-stream`, `vitriol-tui`, `libhermes`, `officina/tui`, `demiurge`.
  - fmt blocking; clippy advisory (owner choice); tests blocking.
- Node: `npm ci`, `npm test` (562 passed, 18 skipped), `npm run typecheck`.
- Python: ruff `E9,F63,F7,F82` + `compileall`.
- Shell: `shellcheck --severity=error scripts/*.sh`.
- Secrets: gitleaks (full history).
- Docs: markdownlint-cli2 on the new core docs.
- Fixed two CI-only issues found on the first run:
  - `setup-rust-toolchain` defaults to `RUSTFLAGS=-D warnings`; set
    `rustflags: ""` so tests are not warning-gated while clippy is advisory.
  - removed the duplicate `Swatinem/rust-cache` step; cache is configured via
    `cache-workspaces` (the duplicate ran `cargo metadata` at repo root and
    failed for `officina/tui`).
- Fixed Python blockers: `VitriolStatus` forward ref → `Dict[str, Any]`;
  removed dead `t0` reference in `long_session_sim.py`.
- Added the missing shebang to `scripts/vitriol-ports.sh` (only shellcheck
  error).
- Added `dependabot.yml`: Cargo per crate, npm (`officina`), GitHub Actions;
  `rebase-strategy: auto`. `engine/` untouched (frozen).

### Repo metadata / community

- Added `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md` (CalVer `2026.11.0`),
  `VERSION`, `CODE_OF_CONDUCT.md`, `.editorconfig`, `.gitattributes`.
- Added PR template and issue templates (bug / feature / config links).
- GitHub settings: description + 10 topics, wiki disabled,
  delete-branch-on-merge enabled, Dependabot alerts + security updates
  enabled, private vulnerability reporting enabled.
- `main` protection: strict required checks (all 13 CI jobs), no force-push,
  no deletion, conversation resolution required, admin bypass retained.

## Verification

- CI run `41516065078` (`8fe876c`): **success** (all 13 jobs).
- `cargo fmt --check`: clean for all formatted crates.
- `cargo test --locked`: green locally for the CI matrix.
- `npm test`: 562 passed / 18 skipped; `npm run typecheck`: clean.
- shellcheck 0.10.0 `--severity=error`: clean.
- markdownlint: 0 issues on the new docs.
- gitleaks: no findings (secret scanning/push protection already enabled).
- Local working tree clean; `main` pushed.

## Follow-ups (not done here)

- Dependabot opened 9 PRs on the first config run; they were rebased after
  the CI fixes. Review/merge as capacity allows.
- Track C (docs drift): 77 files still reference i7-3770 / 1070 Ti / submodule
  era; `AGENTS.md`, `CLAUDE.md`, `docs/optimizations/*` need a machine-facts
  pass.
- Track D (hygiene): tracked scratch (`artifacts/`, `experiments/`,
  `.praetor/shadow-results.json`), stale local/remote branches, and the
  474 MB legacy `.git/modules` (kept while `lull-kv`/`vllm-031` worktrees
  exist).
- Optional: self-hosted CUDA engine smoke workflow; release automation.
