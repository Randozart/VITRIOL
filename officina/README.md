# Officina

The VITRIOL coding workshop's operator frontend, publishable standalone:
a Rust TUI driving [`pi`](https://www.npmjs.com/package/@earendil-works/pi-coding-agent)
coding agents, with 51 extensions for context lifecycle, resilient
model routing, free-cloud lanes, and multi-session operation.

Officina works against **any `llama-server`** (upstream llama.cpp), the
optional **Caduceus** gateway (free-lane aggregation), or the full
**VITRIOL** stack (tuned engine + governor). It does not require
VITRIOL.

```bash
npm install                      # pi + extensions deps
cargo build --release --manifest-path tui/Cargo.toml
./tui/target/release/officina    # the workshop
```

Point it at a backend:

| Backend | How | What you get |
|---|---|---|
| any `llama-server` | `VITRIOL_BASE_URL=http://host:8279` (name is historical) | chat, KV slot save/restore (`--slot-save-path`), session checkpoints |
| Caduceus gateway (optional) | external [OmniRoute](https://github.com/diegosouzapw/OmniRoute) install (MIT), see `THIRD_PARTY_NOTICES.md` | free-lane routing (`officina/nimbus`), overflow, health marking, `/nimbus` |
| full VITRIOL stack | the [VITRIOL repo](https://github.com/Randozart/VITRIOL) | everything: sparse-KV telemetry, governor resize, launcher-wired flags, depth certification |

## What changes without VITRIOL

- `vitriol-decode` telemetry: VITRIOL-specific gauges (sparse-KV
  ejections, ingestion rates) read as 0/unknown — parsers tolerate it.
- Governor integration (`/kv-policy`, `/ctx/resize`): no-op; the
  governor daemon lives in VITRIOL.
- Launcher-wired flags (e.g. `--slot-save-path`) are yours to pass to
  your llama-server; checkpoints then work unchanged.
- Everything else — sessions, scratchpad/tasks, routing lanes,
  free-cloud auto-router, `/nimbus`, snapshots, tree/fork — is fully
  functional.

## Docs

- [`docs/officina-guide.md`](docs/officina-guide.md) — the full operator
  guide (usage, sessions, routing, cache discipline, caduceus).
- Extension-level docs live in each `extensions/<name>/` header comment.

## Configuration

Runtime state lives under `~/.vitriol/` (historical name, kept for
compatibility). Env-file convention: `~/.vitriol/officina/env`. All
Officina env vars are prefixed `OFFICINA_` (kill switches: `*_NO_*=1`,
documented per extension header).

## Layout

- `tui/` — Rust TUI (canonical frontend; spawns `pi --mode rpc`)
- `.pi/extensions/` — the shared extension surface (loaded by pi)
- `runtime/` — vendored-patch build system (`node runtime/build-patch.mjs`)
- `scripts/` — caduceus gateway manager + import/settings tooling
- `docs/` — operator guide

## License

Apache-2.0 OR MIT — see `LICENSE` and `LICENSE-MIT`. Third-party
attributions: `NOTICE`, `THIRD_PARTY_NOTICES.md`.

## Provenance

Development happens in the VITRIOL monorepo (`officina/` subtree);
this repository receives release snapshots. Patches welcome via the
monorepo.
