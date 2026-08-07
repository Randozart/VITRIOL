# Hermetis → Rust port — provenance

Date: 2026-08-07.

## What

The Python memory layer (`libvitriol/hermetis/`, `hermetis_server.py`,
`pymander.py`) is fully replaced by the Rust crate **`libhermes`** with the same
HTTP contracts and (semantic-off) bit-exact behavior. `launch_copula.sh` and
`vitriol pymander` now run the Rust binaries.

## Kind

`user-repo` — a byte-parity port of VITRIOL's own Python memory layer, which is
itself a user-originated design. No third-party source consulted.

## What landed (P1–P5)

- **P1** `libhermes/src/db.rs`: SQLite layer, byte-identical `SCHEMA_DDL`,
  WAL + `synchronous=NORMAL` + `busy_timeout=30000`, single write mutex,
  sessions / episodes (auto turn_index, `follows` edges) / versioned-supersede
  nodes / edges / config / embedding cache. Schema-parity test vs a
  Python-created DB (57 statements byte-identical) + fixture
  `tests/fixtures/python.schema`.
- **P2** `scorer`/`compact`/`hebbian`/`retrieval`: keyword Jaccard, recency,
  composite score, budgeted context, Hebbian edge updates, cascading retrieve.
  Bit-exact semantic-off parity (`tests/parity_p2.rs`); live cross-check
  identical ordering/content vs Python.
- **P3** `server.rs` + `hermes-server` bin (axum): `/health`,
  `/hermetis/{store,node,search,context,recent,stats}` live; `/hermetis/embed`
  503 (no embed provider yet); repo_map/pymander landed in P4/P5. Live parity
  vs Python server: identical JSON.
- **P4** `consolidate.rs` (idle consolidation loop + mark-active middleware)
  and `repomap.rs` (symbol/import regexes, import-graph ranking, budgeted
  Aider-style map, git-rev versioned nodes) + `/hermetis/repo_map`.
- **P5** `pymander.rs` + `pymander` bin (CLI) + `/pymander/{list,search,select,
  context}` routes.

## Behavior notes / parity deltas

- **Semantic-off exact; semantic-on deferred.** The Python server ran with
  `VITRIOL_SEMANTIC_MODE=on` in `launch_copula.sh`; the Rust server has no embed
  provider yet, so `/hermetis/embed` returns 503 and retrieval uses keyword
  scoring. Enabling the semantic path (Rust `embed` provider) is the next phase
  when the GPU embed server is reachable.
- Python quirks preserved for parity: node `strength` defaults to 1.0 on
  ingest; `_edge_weight` cascade tagging; `crate//foo//bar` import tokens; the
  `^\s*` fn pattern shadowing the rust "method" pattern.
- `vitriol_shim.py` still imports `libvitriol.hermetis` under
  `VITRIOL_MEMORY_MODE=on` with a graceful ImportError fallback; it is a legacy
  path — the modern plugin (`copula.ts`) and TUI talk HTTP only.

## Tests

32 tests (db parity, scorer/compact/hebbian/retrieval, server contracts,
consolidate, repomap, pymander) + 2 parity suites. clippy/fmt/Praetor clean.
