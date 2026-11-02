# Changelog

All notable changes to this project are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions use
calendar versioning (`YYYY.MM.PATCH`).

## [Unreleased]

## [2026.11.0] - 2026-11-02

### Added

- In-tree `engine/` monorepo layout (llama.cpp fork at `30b128504`); no submodule.
- Engine provenance record and `engine-import-2026-10-23` tag.
- AVX2/FMA/BMI2 native CPU backend rebuild path; CPU-only bench shows
  ~+74% prefill / ~+79% token generation.
- Profiles: `gemma4-12b-16k`; upstream `qwen38-ud`, `qwen38-ud-262k`.
- GitHub Actions CI: Rust matrix, Node, Python, shellcheck, gitleaks, docs lint.
- Tracked `Cargo.lock` per crate, `package-lock.json`, `rust-toolchain.toml`.
- Repo metadata files: `CONTRIBUTING.md`, `SECURITY.md`, `.editorconfig`,
  `.gitattributes`, issue/PR templates, Dependabot.

### Changed

- `main` is now the canonical monorepo branch; docs updated for the in-tree
  engine (no `git submodule`, no patch step).
- Repo-wide `cargo fmt` pass; formatting is enforced in CI.
- README hardware/flag notes updated for the current dual-3060 host.

### Removed

- Empty `engine/.gitmodules`, stale in-source CMake artifacts, rebis backups,
  and a stray `VITRIOL_EXPERT_COUNT=0` file.

### Fixed

- Officina typecheck: `jiti` is now an explicit dev dependency.
