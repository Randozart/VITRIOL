# Hermetis — VITRIOL's memory system

Date: 2026-08-06.

Hermetis is the persistent RAG memory behind VITRIOL. It gives the OpenCode context
window (via the **Copula** bond) a durable, per-project brain: continuous ingestion,
multi-hop retrieval, consolidation, and whole-repo awareness. The OpenCode plugin that
writes into and reads from Hermetis is the **Copula Hermetis** plugin.

## Architecture

```
libvitriol/hermetis/            the memory engine
  db.py            per-project SQLite (episodes, knowledge_nodes, edges, sessions,
                   config, embeddings), store/search/edge helpers, WAL, write lock
  retrieval.py     multi-hop retrieval: direct search -> edge cascade -> score/rank
  scorer.py        keyword overlap + recency + hebbian + strength scoring;
                   semantic (sentence-transformers) mode when enabled
  compact.py       formatting for context injection (episodes, nodes, symbols)
  consolidate.py   background consolidation: batches -> knowledge nodes + summaries
  hebbian.py       co-retrieval edge-weight reinforcement
libvitriol/hermetis_server.py   the HTTP facade (localhost): /hermetis/store|node|
                   search|stats, /health. Consumed by the Copula Hermetis plugin.
```

## Data model

Per project (`~/.vitriol/<project_id>/memory.db`):
- **episodes** — conversation turns / tool results (role: user, assistant, tool).
- **knowledge_nodes** — durable facts / repo-map entries (label-unique), with a summary.
- **edges** — typed links (follows, consolidated_from, co_retrieved) with weights
  (hebbian reinforcement).
- **sessions** — per-project session state.

## API (HTTP facade)

| endpoint | purpose |
| --- | --- |
| `POST /hermetis/store` | store an episode (role user/assistant/tool) |
| `POST /hermetis/node` | upsert a knowledge node (label-keyed) |
| `POST /hermetis/search` | multi-hop retrieval -> formatted snippets |
| `GET /hermetis/stats` | per-project counts |
| `GET /health` | liveness |

Run: `python3 libvitriol/hermetis_server.py --port 7980`

## Provenance

The engine was re-derived as part of the user's own VITRIOL runtime (AGENTS.md §2.2:
user-owned repos freely borrowable). No third-party code copied. The multi-hop /
cascade retrieval and the scoring blend are original; Aider's repo-map *idea*
(inspiration, not code) informs the P3 map builder.

## Status

- P1 done: server + engine (VITRIOL `cb99d9c`).
- P2 resolved (`f1e62ae`): semantic embeddings via sentence-transformers (all-MiniLM,
  CPU). GGUF-GPU path wired + zero-guarded, gated on a fork BERT-embedding bug (backlog).
- P4: Copula Hermetis plugin (`plugins/copula.ts`) ingests + searches.
- P3 repo map: pending.
