# Copula — the VITRIOL-to-OpenCode bond

Date: 2026-08-06.

Copula is the coupling function between OpenCode and VITRIOL: it makes the OpenCode
context window a *working-memory budget* (measured ~32K fast on this box) and gives
OpenCode a persistent RAG brain — **Hermetis** (VITRIOL's memory system). Named after
the statistical coupling function — two systems, one joint distribution.

## Naming

- **Hermetis** = the memory system (VITRIOL side): the `libvitriol/hermetis` package
  (SQLite per-project store, multi-hop retrieval, scorer) + its HTTP facade
  `libvitriol/hermetis_server.py`. See `docs/hermetis.md`.
- **Copula** = the bond itself (the connector concept).
- **Copula Hermetis** = the OpenCode plugin (`~/.config/opencode/plugins/copula.ts`)
  that connects OpenCode into the Hermetis memory system.

## Components

| component | side | role |
| --- | --- | --- |
| Hermetis server | VITRIOL (`libvitriol/hermetis_server.py`) | HTTP API: `/hermetis/store`, `/hermetis/node`, `/hermetis/search`, `/hermetis/stats`, `/health`. Reuses the `libvitriol/hermetis` spine. |
| Copula Hermetis plugin | OpenCode (`~/.config/opencode/plugins/copula.ts`; versioned at `plugins/copula.ts`) | ingests per-message context (user/assistant/tool results incl. subagents), exposes a `memory_search` tool, injects the repo map at session start. |
| Embedding provider | VITRIOL (sentence-transformers CPU; GGUF-GPU path wired but gated on a fork bug) | all-MiniLM-L6-v2 semantic scoring; GGUF-GPU fallback ready once the fork's BERT bug is fixed. |
| Repo map builder | VITRIOL (P3) | Aider-style: tree-sitter symbols + file-graph rank + token budget. |

## Running the stack

```fish
# 1. Copula memory stack: Hermetis (:8090) + GPU embed server (:8081)
./scripts/launch_copula.sh          # start (COPULA_NO_EMBED=1 to skip embed)
./scripts/launch_copula.sh stop     # stop
# 2. Generation server (separate), e.g. Mellum: ngl=24 c=32768 t=4
# 3. OpenCode picks up the Copula Hermetis plugin at ~/.config/opencode/plugins/copula.ts
#    (copy from VITRIOL/plugins/copula.ts; restart opencode to load it)
```

## Flow

OpenCode events (messages, tool results) -> Copula Hermetis plugin -> Hermetis server ->
SQLite memory (per project). On demand: agent calls `memory_search(query)` -> Hermetis
multi-hop retrieval -> snippets injected back into the window. Whole-repo awareness
comes from the Aider-style repo map, not from growing the window.

## Status

- P1 done (VITRIOL `cb99d9c`): Hermetis server + `store_node` helper + the
  edge-write-commit fix (5s stall) + diff-aware praetor hook.
- P2 resolved (`f1e62ae`): semantic embeddings via sentence-transformers (all-MiniLM,
  CPU). GGUF-GPU path wired + zero-guarded but gated on a fork BERT-embedding bug
  (backlog).
- P4 plugin (`plugins/copula.ts`): ingest (session transcript + tool results) +
  `memory_search` tool; installed at `~/.config/opencode/plugins/copula.ts`.
- P3 repo map, P5 validation: pending — see
  `.opencode/plans/2026-08-06-copula-subsystem.md`.

## Notes

- No proxy/MITM: the plugin talks to the service over loopback HTTP. The legacy
  `vitriol_shim.py` proxy is superseded for the OpenCode path.
- Service binds localhost only.
