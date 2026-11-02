# Security Policy

## Supported versions

Only the current `main` branch is supported. Releases are dated (CalVer);
older tags may not receive fixes.

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Use GitHub's private vulnerability reporting:

<https://github.com/Randozart/VITRIOL/security/advisories/new>

Include:

- affected component (launcher, engine/server, governor, TUI, Officina)
- version/commit
- reproduction steps or proof of concept
- impact and any suggested fix

You should get an acknowledgement within a few days. Please allow time for a
fix before public disclosure.

## Scope

In scope:

- `scripts/vitriol`, `scripts/*.py`, `scripts/*.sh`
- `engine/` server, HTTP API, KV/state persistence, CUDA integration
- `vitriol-governor`, `vitriol-tui`, `libhermes`, `officina/`
- configuration and fingerprint handling

Out of scope:

- vulnerabilities in unmodified upstream llama.cpp code that are already
  fixed upstream (report upstream; we can port a fix)
- denial of service from deliberately malformed local model files
- issues requiring a compromised local account

## Secret handling

The repository never contains `~/.vitriol/secrets`, model files, slot states,
or API keys. Secret scanning and push protection are enabled on GitHub.
If you find a committed secret, report it privately so it can be rotated and
removed from history.
