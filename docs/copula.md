# Copula Subsystem — the VITRIOL-to-OpenCode bond

Date: 2026-08-06.

Copula is the coupling function between OpenCode and VITRIOL: it makes the OpenCode
context window a *working-memory budget* (measured ~32K fast on this box) and gives
OpenCode a persistent RAG brain in VITRIOL. Named after the statistical coupling
function — two systems, one joint distribution.

## Components

| component | side | role |
| --- | --- | --- |
| Copula service | VITRIOL (`libvitriol/copula_server.py`) | HTTP API: `/memory/store`, `/memory/node`, `/memory/search`, `/memory/stats`, `/health`. Reuses the `libvitriol/memory` spine (SQLite per-project, multi-hop retrieval, scorer). |
| Copula plugin | OpenCode (`~/.config/opencode/plugins/copula.ts`) | ingests per-message context (user/assistant/tool results incl. subagents), exposes a `memory_search` tool, injects the repo map at session start. |
| Embedding provider | VITRIOL (P2) | small embedding GGUF via llama-server `/embedding` on GPU (:8081); keyword fallback if VRAM is tight. |
| Repo map builder | VITRIOL (P3) | Aider-style: tree-sitter symbols + file-graph rank + token budget. |

## Flow

OpenCode events (messages, tool results) -> Copula plugin -> Copula service -> SQLite
memory (per project). On demand: agent calls `memory_search(query)` -> Copula service
multi-hop retrieval -> snippets injected back into the window. Whole-repo awareness
comes from the Aider-style repo map, not from growing the window.

## Status

- P1 done (VITRIOL `63c3e5a`): Copula service + `store_node` helper + the
  edge-write-commit fix (5s stall) + diff-aware praetor hook.
- P2 embeddings, P3 repo map, P4 plugin, P5 validation: pending — see
  `.opencode/plans/2026-08-06-copula-subsystem.md`.

## Notes

- No proxy/MITM: the plugin talks to the service over loopback HTTP. The legacy
  `vitriol_shim.py` proxy is superseded for the OpenCode path.
- Service binds localhost only.
