# VITRIOL-Memory for OpenCode — Complementary RAG Architecture

Date: 2026-08-06.

## 1. Goal

Give OpenCode broad-context awareness via a complementary VITRIOL layer: the context
window becomes a working-memory budget (measured: ~32K fast on this box), while
VITRIOL provides the persistent RAG brain — continuous context ingestion, Aider-style
whole-repo map, and on-demand retrieval back into the window. No proxy/MITM: a native
OpenCode plugin (global) talks to a VITRIOL memory service over HTTP.

## 2. Decisions (2026-08-06)

- **Route**: native OpenCode plugin, not the legacy shim proxy.
- **Ingest scope**: conversations AND tool results (file reads, grep, bash) — the real
  "context gathering".
- **Ingest cadence**: per-message, continuous (event-driven), plus capture-before-compaction.
- **Embeddings**: **GGUF on GPU** — a small embedding model via a second llama-server
  instance using the fork's `/embedding` endpoint (server.cpp:191), port 8081.
- **Repo map**: Aider-inspired, but **adaptive to the constrained target** — start
  small (Aider default 1k tokens) and tune the budget to our 32K fast window in P5;
  auto-inject at session start (or retrieval-only if measurement says the map is noise).
- **Subagent/child sessions**: included, keyed to the same project.

## 3. Research anchors

- **OpenCode plugin/SDK** (opencode.ai/docs/plugins, /sdk): plugins get a `client`
  with `event.subscribe()` (SSE of all events), `session.messages()` (full transcript),
  `session.prompt({noReply: true})` (inject context without a response), custom tools
  (`tool()` helper). Events: `message.part.updated`, `tool.execute.after`,
  `session.compacted`, `session.idle`, etc.
- **Aider repo map** (aider.chat/docs/repomap.html): tree-sitter ASTs -> important
  classes/functions with signatures; graph ranking (PageRank on the file-dependency
  graph) selects the most-relevant portion to fit a token budget (`--map-tokens`,
  default 1k); LLM drills into specific files when the map is insufficient.
- **VITRIOL memory spine** (`libvitriol/memory/`): per-project SQLite (`memory.db`),
  episodes/nodes/edges, multi-hop retrieval (`retrieval.py`), relevance/recency/hebbian/
  strength scoring (`scorer.py`), consolidation (`consolidate.py`), compaction
  (`compact.py`), vector-store semantic mode (`vector_store.py`).
- **Embedding endpoint**: fork llama-server has `/embedding` + `/embeddings`
  (tools/server/server.cpp:191), gated on embedding mode; legacy shim exposed
  `/memory/stats`, `/memory/clear`, `/context/archive|retrieve` — but NO clean
  store/search API (this plan adds it).
- **Measured context budget** (this session): 32K fast / 131K slow (KV offload);
  prefill incremental via prompt caching; Mellum uses ~7 G of 8 G VRAM -> embedding
  VRAM contention is a flag.

## 4. Architecture

```
OpenCode (global plugin ~/.config/opencode/plugins/vitriol-memory.ts)
  |- ingest:    event.subscribe() -> per-message store (user, assistant, tool results)
  |- retrieve:  custom tool memory_search(query) -> VITRIOL /memory/search
  |- repo map:  auto-inject budget-limited Aider-style map at session start
                              |
                              v
             VITRIOL Memory Service (libvitriol/memory_server.py)
               POST /memory/store  /memory/search  /memory/embed  /memory/repo_map
                              |
                              v
             Embedding provider: llama-server, small embedding GGUF, GPU, :8081
                              |  (reuses libvitriol.memory db/retrieval/scorer)
```

## 5. Components

### 5.1 VITRIOL memory service (`libvitriol/memory_server.py`)
- FastAPI/Flask, localhost-bound.
- `POST /memory/store {project, type: episode|node, content, meta, session}`
- `POST /memory/search {project, query, top_k}` -> multi-hop retrieval (reuse
  `retrieval.py`), semantic mode via embeddings when available, keyword fallback.
- `POST /memory/embed {text}` -> calls the embedding provider (llama-server :8081).
- `GET /memory/repo_map {project, budget_tokens}` -> Aider-style map.
- Reuses `libvitriol/memory/*` unchanged (db, retrieval, scorer, consolidate, compact).
- Supersedes the legacy shim for the OpenCode path.

### 5.2 Embedding provider (GPU, GGUF)
- Second llama-server instance, small embedding GGUF (e.g. all-MiniLM / nomic-embed
  style), `--embedding`, GPU, port 8081.
- VRAM guard: if free VRAM < embedding footprint, fall back to keyword+recency
  scoring (the current non-semantic path) — never starve the generation server.
- Verify `/embedding` works in the fork on the chosen GGUF in P2.

### 5.3 Aider-style repo map builder
- tree-sitter symbol extraction -> per-file signatures (classes, functions).
- File-dependency graph (imports) -> PageRank -> importance rank.
- Budget-limited map (start 1k tokens, tune in P5 to the 32K fast window).
- Stored as memory nodes; refreshed on file change (file.watcher / on-demand).

### 5.4 OpenCode plugin (global)
- **Ingest**: `event.subscribe()` -> on `message.part.updated` (assistant), user
  messages, and `tool.execute.after` (tool results) -> POST /memory/store. Keyed by
  project directory + session. Includes child/subagent sessions.
- **Retrieve**: custom tool `memory_search(query)` -> /memory/search -> snippets.
- **Repo map**: on session start, inject the budget-limited map via
  `session.prompt({noReply: true})` (adaptive: keep if it helps, drop if noise).

## 6. Phases

- **P1** — Memory service (store/search) on the existing spine; unit-test retrieval.
- **P2** — GPU embedding provider (llama-server /embedding + small GGUF) + wire into
  the service, with VRAM-contention guard + keyword fallback.
- **P3** — Aider-style repo map builder (tree-sitter symbols + graph rank + budget).
- **P4** — OpenCode plugin (ingest hooks + `memory_search` tool + map injection).
- **P5** — End-to-end validation: session -> RAG -> retrieval -> context loop; measure
  window-size impact + prefill (prompt caching); tune repo-map budget.

## 7. Risks / open flags

- **VRAM contention**: Mellum uses ~7 G of 8 G; embedding instance must fit the
  remainder or run degraded (keyword fallback). Measure in P2.
- **Embedding GGUF compatibility**: fork `/embedding` may need a specific model/arch;
  verify in P2 before committing to a model.
- **Repo map budget**: too big wastes the 32K fast window; too small is noise. A/B in
  P5.
- **Ingest volume**: per-message storing of large tool results could bloat the DB —
  cap stored content size, rely on consolidation.
- **Security**: service binds localhost only; no auth needed for local use.

## 8. Deliverables

- Memory service + embedding provider + repo map builder (VITRIOL).
- Global OpenCode plugin (ingest + search + map).
- Per-project DB via the existing spine; docs + provenance in both repos.
