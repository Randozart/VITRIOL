# Copula Subsystem — the VITRIOL-to-OpenCode bond (Hermetis memory)

Date: 2026-08-06.

## 1. Goal

Give OpenCode broad-context awareness via a complementary VITRIOL layer: the context
window becomes a working-memory budget (measured: ~32K fast on this box), while
VITRIOL provides the persistent RAG brain — **Hermetis**, the memory system. No
proxy/MITM: a native OpenCode plugin (global) talks to the Hermetis HTTP service over
loopback. This VITRIOL-to-OpenCode bond is the **Copula subsystem** (a coupling
function, named 2026-08-06). Naming:
- **Hermetis** = the memory system (VITRIOL side): `libvitriol/hermetis` package +
  `libvitriol/hermetis_server.py`.
- **Copula** = the bond/connector concept.
- **Copula Hermetis** = the OpenCode plugin that connects OpenCode into Hermetis.

## 2. Decisions (2026-08-06)

- **Name**: VITRIOL<->OpenCode integration = **Copula subsystem**; memory system =
  **Hermetis**; the OpenCode plugin = **Copula Hermetis**.
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
- **Hermetis memory engine** (`libvitriol/hermetis/`): per-project SQLite (`memory.db`),
  episodes/nodes/edges, multi-hop retrieval (`retrieval.py`), relevance/recency/hebbian/
  strength scoring (`scorer.py`), consolidation (`consolidate.py`), compaction
  (`compact.py`), vector-store semantic mode.
- **Embedding endpoint**: fork llama-server has `/embedding` + `/embeddings`
  (tools/server/server.cpp:191), gated on embedding mode; legacy shim exposed
  `/memory/stats`, `/memory/clear`, `/context/archive|retrieve` — but NO clean
  store/search API (this plan adds it).
- **Measured context budget** (this session): 32K fast / 131K slow (KV offload);
  prefill incremental via prompt caching; Mellum uses ~7 G of 8 G VRAM -> embedding
  VRAM contention is a flag.

## 4. Architecture

```
OpenCode — Copula Hermetis plugin (global ~/.config/opencode/plugins/copula.ts)
  |- ingest:    event.subscribe() -> per-message store (user, assistant, tool results)
  |- retrieve:  custom tool memory_search(query) -> Hermetis /hermetis/search
  |- repo map:  auto-inject budget-limited Aider-style map at session start
                              |
                              v
             Hermetis server (libvitriol/hermetis_server.py)
               POST /hermetis/store  /hermetis/search  /hermetis/embed  /hermetis/repo_map
                              |
                              v
             Embedding provider: llama-server, small embedding GGUF, GPU, :8081
                              |  (reuses libvitriol/hermetis db/retrieval/scorer)
```

## 5. Components

### 5.1 Hermetis server (`libvitriol/hermetis_server.py`)
- Flask, localhost-bound.
- `POST /hermetis/store {project, type: episode|node, content, meta, session}`
- `POST /hermetis/search {project, query, top_k}` -> multi-hop retrieval (reuse
  `retrieval.py`), semantic mode via embeddings when available, keyword fallback.
- `POST /hermetis/embed {text}` -> calls the embedding provider (llama-server :8081).
- `GET /hermetis/repo_map {project, budget_tokens}` -> Aider-style map.
- Reuses `libvitriol/hermetis/*` unchanged (db, retrieval, scorer, consolidate, compact).
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

### 5.4 Copula Hermetis plugin (global)
- **Ingest**: `event.subscribe()` -> on `message.part.updated` (assistant), user
  messages, and `tool.execute.after` (tool results) -> POST /hermetis/store. Keyed by
  project directory + session. Includes child/subagent sessions.
- **Retrieve**: custom tool `memory_search(query)` -> /hermetis/search -> snippets.
- **Repo map**: on session start, inject the budget-limited map via
  `session.prompt({noReply: true})` (adaptive: keep if it helps, drop if noise).

## 6. Phases

- **P1 — DONE (2026-08-06, VITRIOL cb99d9c)** — Hermetis server `libvitriol/hermetis_server.py`
  (store/node/search/stats/health, localhost :8090). Reuses the existing spine. Bugs
  found + fixed while building:
  - `store_episode` left the edge INSERT (`_ensure_edge`) uncommitted → open write
    transaction held the SQLite write lock → 3rd sequential store stalled ~5 s under
    threaded Flask (`busy_timeout`). Fixed: commit after edge link.
  - Param bundling per AGENTS.md 5.3: `db.EdgeSpec` dataclass; `store_episode` /
    `store_node` take a `meta` dict; callers updated (hebbian, consolidate, shim).
  - Pre-commit hook upgraded to **diff-aware** (`scripts/praetor_diff.py`): only NEW
    diagnostics relative to the staged version gate the commit (pre-existing baseline
    in touched files no longer blocks).
  - Verified: 5 sequential stores + node all ~0.003 s; search returns scored formatted
    snippets.
- **P2** — GPU embedding provider (llama-server /embedding + small GGUF) + wire into
  the service, with VRAM-contention guard + keyword fallback.
- **P2 — BLOCKED on the fork's BERT embedding bug (2026-08-06)**. GPU-GGUF embedding
  provider: `/embedding` + `--pooling` verified present, `nomic-embed-text-v1.5` Q8_0/F16
  and `bge-small-en-v1.5` Q8_0 downloaded and served. BUT both BERT-family models return
  **all-zero embeddings for many common inputs** ("fast", "how do we sort a list fast",
  "Write a Python function for merge sort" -> norm 0.0; "hello world" -> norm 1.0).
  Reproduces on GPU (-ngl 99) AND CPU (-ngl 0), and under `--pooling cls` / `mean` /
  default — backend- and pooling-independent. Conclusion: a fork regression in the
  BERT-family embedding forward pass (the fork heavily modified attention/KV/buffers).
  sentence-transformers is NOT installed on this box (the CPU fallback is unavailable).
  Mitigation added: **zero-guard** in `hermetis/embed.py` (near-zero vector -> None ->
  keyword fallback) so Hermetis semantic scoring never uses poisoned zero vectors.
- **P2 — RESOLVED 2026-08-06 (decision): sentence-transformers CPU** (`pip install
  sentence-transformers`, all-MiniLM-L6-v2). Rationale (proposal approved): the GGUF-GPU
  path is gated behind a fork bug; sentence-transformers is the designed fit (scorer.py
  already targets all-MiniLM-L6-v2 384-dim) and needs no rewiring; CPU is ms-scale for
  short-text embedding and avoids VRAM contention with the gen server. The GGUF-GPU path
  stays wired + zero-guarded for later. **Fork BERT-embedding bug -> BACKLOG**: git-bisect
  the fork's attention/graph changes to isolate the regression (own investigation, not on
  Copula's critical path).
- **P3** — Aider-style repo map builder (tree-sitter symbols + graph rank + budget).
- **P4 — DONE (2026-08-06)** — Copula Hermetis plugin (`plugins/copula.ts`, installed
  `~/.config/opencode/plugins/copula.ts`): event-hook ingest of user/assistant text
  (session transcript on idle) + `tool.execute.after` tool-result capture, deduped;
  `memory_search` custom tool -> Hermetis `/hermetis/search`. End-to-end verified:
  store user/assistant/tool -> semantic search ranks the relevant episode (0.892).
  Restart opencode to load the global plugin.
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

- Hermetis server + embedding provider + repo map builder (VITRIOL).
- Copula Hermetis plugin (ingest + search + map).
- Per-project DB via the existing spine; docs + provenance in both repos.
