# Contributing

VITRIOL is a single-author research project that welcomes focused issues and
pull requests. The engine is a deeply modified llama.cpp fork; treat it as
first-party code, not an upstream submodule.

## Ground rules

- `main` is canonical. Work happens directly on `main` or in short-lived branches.
- **Never sync `engine/` from upstream.** The fork is frozen; port a specific
  security or model-support fix with a `PROVENANCE:` note instead.
- Never commit models, `~/.vitriol` state, slot files, API keys, or tokens.
- Speed-bearing flag changes must record a fingerprint diff. See
  `AGENTS.md` ("Flag provenance") before touching launcher/config flags.
- Experiments get a timestamped report under `.opencode/plans/`; do not leave
  findings only in chat logs.

## Setup

Requirements:

- Linux, g++ 16.2 for VITRIOL `.cpp` (CUDA host compiler may be older),
  CUDA toolkit for the engine
- Rust via `rust-toolchain.toml`
- Node 22 for `officina/`
- Python 3.12 for scripts/tests

```bash
git clone https://github.com/Randozart/VITRIOL.git
cd VITRIOL

# Engine (out-of-source build; VITRIOL patches are already in engine/)
CUDA_ARCHITECTURES=86 ./scripts/build-llama-server.sh --backend cuda

# Rust
cargo test --locked --manifest-path vitriol-governor/Cargo.toml

# Officina
cd officina && npm ci && npm test && npm run typecheck
```

## Checks before pushing

```bash
# Rust crates (the CI matrix)
cargo fmt --check
cargo clippy --locked --all-targets
cargo test --locked

# Shell
shellcheck --severity=error scripts/*.sh

# Python
ruff check --select E9,F63,F7,F82 scripts tests libvitriol
python -m compileall -q scripts tests libvitriol
```

Run the commands from each crate directory. CI runs the same matrix plus
Node, Python, shell, secret scanning, and docs lint.

## Style

- C++ host code follows `docs/cpp26-conventions.md`.
- Rust: `cargo fmt` is authoritative.
- Shell: keep `scripts/vitriol` POSIX-ish bash and shellcheck-clean at error level.
- Commit messages: Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`,
  `ci:`, `style:`). One logical change per commit.
- Every new algorithm-bearing module gets a `PROVENANCE:` header.

## Reporting

- Bugs and feature requests: GitHub issues.
- Security issues: **do not open a public issue** — see `SECURITY.md`.
