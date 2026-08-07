# Hermetis memory subsystem → Rust port

Date: 2026-08-07.

## 1. Goal

Port the Python memory layer to a single Rust crate — `db`, `retrieval`,
`scorer`, `embed`, `compact`, `hebbian`, `consolidate`, `repomap`,
`hermetis_server.py`, and `pymander.py` — keeping **identical HTTP contracts**
so `plugins/copula.ts` and `vitriol-tui` are untouched. Aligns with AGENTS.md
§5.8 "Rust is preferred for tooling, calibration, and host-side code."

## 2. Scope

| provided by | Python (now) | Rust (target) |
|---|---|---|
| DB store (schema + WAL + write lock) | `hermetis/db.py` | crate `libhermetis` (rusqlite) |
| retrieval / scorer / compact / hebbian / embed | `hermetis/{retrieval,scorer,compact,hebbian,embed}.py` | pure Rust (`regex`+serde) |
| HTTP server `/hermetis/*`,`/pymander/*`,`/health` | `hermetis_server.py` | `axum` + `tokio` |
| repo map (symbol extract + git) | `hermetis/repomap.py` | crate (`regex` +`std::process`) |
| consolidation thread | `hermetis/consolidate.py` | `tokio::task` idle timer |
| Pymander CLI | `libvitriol/pymander.py` + `pymander_test.py` | crate subcommand (`clap`) |

Also removed (dead/legacy, none on the modern `launch_vitriol_full.sh` path):
`libvitriol/vector_store.py`, `libvitriol/vitriol_types.py`,
`libvitriol/vitriol_layer_manager.py`, stale `scripts/launch_vitriol_v2.sh`,
and the Python `gguf_reader.py` drift (Rust `gguf.rs` already primary).

Bed/rewiring: `scripts/launch_copula.sh` and `vitriol pymander` point to the Rust
binary in place of the Python module.

## 3. Parity is the gate

- Semantic-off path = regex/Jaccard + arithmetic → **bit-exact** parity.
- Semantic-on calls the same GPU embed server; same input string → same vector →
  same score (tolerance, never `==` on floats; AGENTS §10.1).
- DB lives at the same `~/.vitriol/<project_id>/memory.db` / pymander path → schema
  is byte-compared, existing data stays readable.

### Parity harness `scripts/hermes ~_parity.sh`
Runs old-Python vs new-Rust servers on identical sequences against two temp memory
roots; diffs `/hermes/stats`, `/hermes/search`, `/hermes/context`,
`/hermes/recent`, `/pymander/*` JSON (semantic-off exact; semantic-on tolerance).
Plus `sqlite3 .schema` diff in both engines.

Port `pymander_test.py` (12 cases) → Rust `#[test]`s (identical fixtures, node/
supersede/selection behavior, search ordering).

## 4. Phasing (each commit green)

- **P0** — reject legacy deletions; confirm none on live path (verified).
- **P1** — Rust DB layer: schema, store_node/episode, edges, sessions, config,
  embeddings, versioned supersede, WAL. Schema-parity test.
- **P2** — retrieval/scorer/compact/hebbian (bit-exact, semantic-off).
- **P3** — axum server: all `/hermes/*` + `/pymander/*` routes; parity vs Python
  responses.
- **P4** — consolidation thread + repomap.
- **P5** — Pymander CLI (list/ingest/nodes/search/select/active/doctrine/promote).
- **P6** — delete Python memory modules, repoint launchers, docs/provenance
  (`docs/provenance/hermes-rust.md`), AGENTS update.

## 5. Non-$targets (explicit, not this effort)

- TS plugin (`copula.ts`) — opencode SDK is JS/TS. Not Rust.
- C kernel module, CUDA/C++ kernels, `llama.cpp` submodule — inherently C/C++/CUDA.
- bash orchestration glue — stays.

## 6. Risks / tradeoffs

- Real 5k-LOC port; parity harness is the net, but live `context` edge cases
  (`is_new_top embed`) need GPU-free check once the card frees.
- Threaded writes mirror Python's global-write-lock to avoid sqlite lock
  regressions (observe Python's `_write_lock` pattern + busy_timeout).
- Consolidation touches shared state; port second with the same env knobs.
- If a parity case is not reducible bit-exact, STOP and record in BUGS.md + plan —
  contract is the truth, fix the code not the contract (AGENTS §3).

## 7. Results

- **P1 landed**: crate `libhermes` (rusqlite bundled, sha2). `db.rs` ports the
  full DB layer byte-for-byte: SCHEMA_DDL (identical to Python `_init_db`),
  WAL + `synchronous=NORMAL` + `busy_timeout=30000`, single write mutex
  (Python `_write_lock`), `get_or_create_session`, `store_episode` (auto
  turn_index, turn_count bump, `follows` edge, committed edge write),
  `store_node` (versioned supersede, never hard-discard), fetch/search shapes,
  `edge_targets` (explicit UNION column lists), `get_or_create_edge`/
  `update_edge_weight`, `config`, and the embeddings cache
  (`content_hash` = sha256 hex, get/store blob).
- **Schema parity verified**: Rust-created DB's `sqlite_master` is byte-identical
  to a Python-created DB (57 statements) — permanent fixture test
  `tests/fixtures/python.schema` + `tests/parity.rs` (byte-parity + readback of
  Python-shaped data). 7 tests green, clippy/fmt/Praetor clean.
- **P2 landed**: `scorer.rs` (estimate_tokens, keyword_overlap Jaccard, recency
  decay, composite `compute_score` with bundled `ScoreInput`/`ScoreWeights`),
  `compact.rs` (format_episode/node/compact + budgeted `compact_context` with
  `CompactOptions`), `hebbian.rs` (is_referenced, update_weights split into
  per-candidate/per-pair helpers), `retrieval.rs` (classify_intent,
  `retrieve` pipeline split hop1/cascade/score, `context_block` with
  `ContextBlockOptions`). Semantic-off is bit-exact: `tests/parity_p2.rs`
  asserts keyword_overlap/estimate_tokens/formatters match captured Python
  values exactly. Live cross-check vs Python `retrieve` on the same fixture:
  identical ordering + content (score drift only from the time-dependent
  recency term). 24 tests, clippy/fmt/Praetor clean (Praetor-driven refactors:
  param bundling, early returns, single-loop cascade decomposition).
- **P3 landed** (`49ead5e`): axum server (`hermes-server` binary). `/health`,
  `/hermetis/{store,node,search,context,recent,stats}` live with Python-parity
  JSON contracts (live cross-check: identical search results/ordering);
  `/hermetis/embed` → 503 (semantic-off); repo_map/pymander 501 until P4/P5.
  Route-handler tests via tower oneshot.
- **P4 landed** (`885aaf3`): `consolidate.rs` (unconsolidated-batch detection,
  deterministic summary, node+`consolidated_from` edges, node decay, retention
  prune; `ConsolidationWorker` idle loop wired into the server with a
  mark-active middleware) + `repomap.rs` (per-lang symbol/import regexes,
  import-graph in-degree ranking, budgeted Aider-style map, git-rev versioned
  node storage) + `/hermetis/repo_map` route. 32 tests,
  clippy/fmt/Praetor clean (Praetor-driven single-loop decomposition).
- Next: P5 (Pymander CLI + `/pymander/*`), P6 (delete Python memory modules +
  repoint launchers + provenance).
