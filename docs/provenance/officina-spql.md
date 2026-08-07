# Officina / Alka (SPQL) — provenance

Date: 2026-08-07.

## What

`vitriol-tui/src/officina/` — the Officina REPL tab: a transactional model-surgery
workshop where the model is treated as a queryable database. Commands are
keyword-first, pipe-delimited (`[COMMIT >] KEYWORD > target args`), probe by
default, commit via `COMMIT >` prefix. Ships DESCRIBE (GGUF metadata census),
TEST (live gen server), MAP (system memory), COMPILE (`.spagyr` bundle),
RECORD/STOP/PLAY (grimoire recipe files), UNDO, CLEAR, HELP.

## Kind

`paper-spec` / public-concept, re-derived. The "the model is the database"
paradigm is a public idea from **LARQL** (2026): neural-network weights treated
as rows in a relational database, editing as composable operations. Re-implemented
independently in Rust — no LARQL code consulted or copied.

## Naming

The name **Alka** is repurposed from the user's own (now-dropped) `alka-lang`
project, per user instruction 2026-08-07. No code, grammar, or design is borrowed
from it; its historical docs are untouched and are not the source of this module.

## Building blocks

- `vitriol-calibrate` (user repo, `libvitriol/src/gguf.rs`): `read_gguf`
  metadata census for DESCRIBE — user-owned, borrowed freely.
- Hermetis/gen snapshot (user repo): telemetry + journal sidebar data.
- Grammar, grimoire, config: fresh.

## Status

P0+P1 landed (shell + grammar + probe/commit + grimoire + COMPILE bundle).
P2 (tensor catalog) and P3 (offline rewrite) planned.
