# VITRIOL Emulated Memory Architecture

> **Date**: 2026-05-17
> **Status**: Design complete — MVP implementation in progress
> **See also**: `OPTIMIZATION_PLAN.md` (Phase 1, #4), `CONTEXT_OFFLOADING_STRATEGIES.md`
> **Config toggle**: `VITRIOL_MEMORY_MODE` — `on` | `off` (default: `off`)

---

## Table of Contents

1. [Motivation](#motivation)
2. [Architecture Overview](#architecture-overview)
3. [Database Schema](#database-schema)
4. [The Core Loop](#the-core-loop)
5. [Scoring Function](#scoring-function)
6. [Spreading Activation (Cascading Retrieval)](#spreading-activation-cascading-retrieval)
7. [Token-Budgeted Compaction](#token-budgeted-compaction)
8. [Hebbian Weight Updates](#hebbian-weight-updates)
9. [Memory Consolidation ("Sleep")](#memory-consolidation-sleep)
10. [OpenCode Integration](#opencode-integration)
11. [Implementation Phases](#implementation-phases)
12. [File Layout](#file-layout)
13. [Configuration Reference](#configuration-reference)

---

## Motivation

OpenCode (and similar agentic coding tools) manage their own context window by compacting — trimming older messages, summarizing, and rewriting context. On a PCIe-bottlenecked system, this compaction loop is expensive:

- Every compaction triggers a prefill of the compacted context
- On MoE models with expert offloading, prefill runs at 20–30 tok/s over PCIe
- At 24K tokens of context, that's 800–1200 seconds of prefill per compaction
- OpenCode compacts around 50% usage → half the conversation is spent compacting

**Solution**: Move context management *outside* the inference engine. Intercept the prompt before it reaches llama.cpp, retrieve only the most relevant context from a persistent project-local memory database, and inject a compact, high-signal context window. The engine never sees a bloated prompt, never needs to compact.

---

## Architecture Overview

```
OpenCode / Agent
  │  POST /v1/chat/completions
  │  Headers: X-Project-Id, X-Session-Id
  ▼
┌──────────────────────────────────────────────┐
│         VITRIOL Context Router                │
│  (vitriol_shim.py — Flask proxy)              │
│                                               │
│  1. Parse headers → select .vitriol/<id>/     │
│  2. Extract user intent from last message     │
│  3. Query memory DB (scoring + cascade)       │
│  4. Inject retrieved context as system msg    │
│  5. Forward compact prompt to llama.cpp       │
└────┬─────────────────────────────────────┬────┘
     │                                     │
     ▼                                     ▼
┌──────────────┐                  ┌──────────────────┐
│  .vitriol/   │                  │  llama.cpp        │
│  memory.db   │                  │  (llama-server)   │
│  (SQLite)    │                  │  -c 8192          │
└──────────────┘                  └──────────────────┘
     │
     │ Background:
     ▼
┌──────────────────┐
│ Consolidation    │
│ (offline thread) │
└──────────────────┘
```

---

## Database Schema

Every project gets its own `.vitriol/memory.db` — created automatically on first request with `X-Project-Id`.

### Episodes — Raw Conversation Turns

```sql
CREATE TABLE episodes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    turn_index   INTEGER NOT NULL,
    role         TEXT NOT NULL,          -- 'user' | 'assistant' | 'system'
    content      TEXT NOT NULL,
    token_count  INTEGER DEFAULT 0,
    created_at   TEXT DEFAULT (datetime('now'))
);
CREATE INDEX idx_episodes_session ON episodes(session_id, turn_index);
CREATE INDEX idx_episodes_created ON episodes(created_at);
```

### Knowledge Nodes — Consolidated Summaries

```sql
CREATE TABLE knowledge_nodes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    label        TEXT NOT NULL UNIQUE,
    summary      TEXT NOT NULL,
    source_min   INTEGER,                -- earliest episode.id
    source_max   INTEGER,                -- latest episode.id
    strength     REAL DEFAULT 1.0,       -- decays to 0.3 floor
    created_at   TEXT DEFAULT (datetime('now'))
);
```

### Graph Edges — Associations + Hebbian Weights

```sql
CREATE TABLE edges (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    from_type    TEXT NOT NULL,           -- 'episode' | 'node' | 'symbol'
    from_id      INTEGER NOT NULL,
    to_type      TEXT NOT NULL,
    to_id        INTEGER NOT NULL,
    relation     TEXT NOT NULL,           -- 'follows' | 'references' | 'consolidated_from' | 'co_retrieved'
    weight       REAL DEFAULT 1.0,
    updated_at   TEXT DEFAULT (datetime('now')),
    UNIQUE(from_type, from_id, to_type, to_id, relation)
);
```

### Symbols — Code Definition Index (Phase 2)

```sql
CREATE TABLE symbols (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL,           -- 'function' | 'struct' | 'trait' | 'module'
    file_path    TEXT NOT NULL,
    line_start   INTEGER,
    line_end     INTEGER,
    signature    TEXT,                    -- compact one-line signature
    doc_comment  TEXT
);
CREATE INDEX idx_symbols_name ON symbols(name);
```

### Sessions — Per-Session Metadata

```sql
CREATE TABLE sessions (
    session_id   TEXT PRIMARY KEY,
    label        TEXT,
    turn_count   INTEGER DEFAULT 0,
    created_at   TEXT DEFAULT (datetime('now')),
    updated_at   TEXT DEFAULT (datetime('now'))
);
```

### Configuration — Per-Project Overrides

```sql
CREATE TABLE config (
    key   TEXT PRIMARY KEY,
    value TEXT
);
```

---

## The Core Loop

```
STEP 1 — INTERCEPT
  Read X-Project-Id → select .vitriol/<project>/memory.db
  Read X-Session-Id → select or create session row
  Extract last user message as query Q

STEP 2 — ANALYZE INTENT
  Keyword-based classification (Phase 1):
    - "fix", "bug", "error", "crash" → code_debug
    - "how", "what", "why", "explain" → question
    - "add", "implement", "create", "refactor" → code_write
    - default → general

STEP 3 — RETRIEVE (with cascading)
  Hop 1: Search episodes + knowledge_nodes by recency and keyword overlap
  Hop 2: For each result, traverse edges → fetch neighbors
  Score all candidates via scoring function
  Sort by score descending

STEP 4 — COMPACT (token budget)
  active_budget = sessions.token_budget (default 4000 tokens)
  Inject session label + current query
  For each candidate:
    if budget has room: inject full content
    else: inject signature / truncated summary
  Stop when budget exhausted or no more candidates

STEP 5 — INJECT
  Format as system message with delimiters
  Prepend to messages array
  Forward to llama.cpp

STEP 6 — STORE (after response)
  Store user message + assistant response as episodes
  Link episodes with edges (relation='follows')
  Update session turn_count

STEP 7 — CONSOLIDATE (background, every N turns or idle)
  Summarize batches of episodes into knowledge nodes
  Create consolidation edges
  Decay unused node strengths
```

---

## Scoring Function

```
score = (relevance × 0.4) + (recency × 0.35) + (hebbian × 0.15) + (strength × 0.10)

where:
  relevance  = keyword_overlap(query, content)     — Jaccard similarity of word sets
  recency    = 1.0 - (days_old / 30.0)             — linear decay over 30 days, clamped 0–1
  hebbian    = avg(edge.weight for co_retrieved)    — only if edges exist, else 0.5
  strength   = knowledge_nodes.strength             — 1.0 for episodes, 0.3–1.0 for nodes
```

All components are normalized to [0, 1]. Component weights are configurable.

---

## Spreading Activation (Cascading Retrieval)

```
Hop 0: Query Q
  │
  ▼
Hop 1: Direct retrieval
  ├─ episodes matching Q → [E1, E3]
  ├─ nodes matching Q   → [N2]
  └─ symbols matching Q → [S5]
      │
      ▼
Hop 2: Follow edges
  ├─ from (episode, E1)  → follows → (episode, E2)   → add E2
  ├─ from (episode, E3)  → references → (symbol, S5) → already have S5
  ├─ from (node, N2)     → consolidated_from → episodes [E4, E5, E7] → add E4, E5, E7
  └─ from (symbol, S5)   → co_retrieved → (symbol, S8) → add S8
      │
      ▼
Hop 3: Follow edges again (configurable depth)
  ├─ from (episode, E2)  → ...
  └─ ...

Final candidate set: {E1, E2, E3, E4, E5, E7, N2, S5, S8}
De-duplicate, score, rank, compact.
```

---

## Token-Budgeted Compaction

```python
def compact(candidates, budget: int) -> list[str]:
    """Convert candidate memories to injected text, respecting token budget."""
    injected = []
    tokens_used = 0

    # Always inject current session context first
    session_context = format_session_header()
    injected.append(session_context)
    tokens_used += estimate_tokens(session_context)

    for candidate in candidates:
        if tokens_used >= budget:
            break

        if candidate.type == 'episode':
            text = format_episode(candidate)
        elif candidate.type == 'node':
            text = format_node(candidate)
        elif candidate.type == 'symbol':
            text = format_symbol_signature(candidate)
        else:
            continue

        tokens = estimate_tokens(text)
        if tokens_used + tokens <= budget:
            injected.append(text)
            tokens_used += tokens
        else:
            # Budget exhausted — inject compact signature instead
            sig = format_compact(candidate)
            sig_tokens = estimate_tokens(sig)
            if tokens_used + sig_tokens <= budget:
                injected.append(sig)
                tokens_used += sig_tokens

    return injected
```

### Injection Format

```
<|im_start|>system
[Memory Context — VITRIOL Emulated Memory]
Project: {project_id} | Session: {session_id}

## Recent Context
user: What's the null pointer in connection_handler?

## Relevant Past Episodes
[2026-05-17] user: How does socket_connect handle timeouts?
[2026-05-17] assistant: socket_connect() has a 30s default timeout…

## Relevant Symbols
fn socket_connect(addr: SocketAddr) -> Result<TcpStream>
// Establishes TCP connection with configurable timeout
<|im_end|>
```

---

## Hebbian Weight Updates

After the LLM responds, the shim updates edge weights:

```python
def update_hebbian_weights(response_text: str, retrieved: list[Candidate]):
    """
    1. Check which retrieved items the LLM actually used in its response
    2. Strengthen edges between co-retrieved items that were used
    3. Weaken edges for items that were retrieved but unused
    """
    for candidate in retrieved:
        used = candidate.name in response_text or candidate.id in response_text

        for edge in get_edges(from_type=candidate.type, from_id=candidate.id):
            if used:
                edge.weight = min(3.0, edge.weight + 0.1)
            else:
                edge.weight = max(0.0, edge.weight - 0.05)

        # Strengthen co-retrieved pairs
        for other in retrieved:
            if other is candidate:
                continue
            used_together = used and (other.name in response_text)
            edge = get_or_create_edge(
                candidate.type, candidate.id,
                other.type, other.id,
                'co_retrieved'
            )
            if used_together:
                edge.weight = min(3.0, edge.weight + 0.1)
            else:
                edge.weight = max(0.0, edge.weight - 0.02)
```

Over time, the graph learns which memories are frequently retrieved together and used, creating personalized "trains of thought."

---

## Memory Consolidation ("Sleep")

Runs as a background thread when the shim is idle (no requests for >60s):

```python
def consolidate():
    """
    1. Find batches of 50 consecutive episodes not yet consolidated
    2. Concatenate them with a summarization prompt
    3. Send to a tiny local model (or CPU-only inference)
    4. Create a knowledge_node with the summary
    5. Create edges from node to all source episodes
    6. Delete episodes older than retention threshold
    """
    unconsolidated = db.execute("""
        SELECT e.* FROM episodes e
        LEFT JOIN edges on edges.to_id = e.id AND edges.relation = 'consolidated_from'
        WHERE edges.id IS NULL
        ORDER BY e.id
        LIMIT 50
    """)

    if len(unconsolidated) < 10:
        return  # not enough to consolidate

    summary = generate_summary(unconsolidated)  # via small local model

    node_id = db.execute("""
        INSERT INTO knowledge_nodes (label, summary, source_min, source_max)
        VALUES (?, ?, ?, ?)
    """, [f"consolidated_{unconsolidated[0].id}",
          summary,
          unconsolidated[0].id,
          unconsolidated[-1].id])

    for ep in unconsolidated:
        db.execute("""
            INSERT INTO edges (from_type, from_id, to_type, to_id, relation)
            VALUES ('node', ?, 'episode', ?, 'consolidated_from')
        """, [node_id, ep.id])

    # Decay old node strengths
    db.execute("""
        UPDATE knowledge_nodes
        SET strength = MAX(0.3, strength * 0.95)
        WHERE created_at < datetime('now', '-7 days')
    """)
```

---

## OpenCode Integration

OpenCode sends custom headers with every API request. The shim reads them to route memory.

### Configuration

In OpenCode's config file (`opencode.json`):

```json
{
  "customHeaders": {
    "X-Project-Id": "vitriol-engine",
    "X-Session-Id": "${workspaceFolderHash}"
  }
}
```

### Header Processing

| Header | Example | Purpose |
|--------|---------|---------|
| `X-Project-Id` | `vitriol-engine` | Selects `.vitriol/<id>/memory.db` — project isolation |
| `X-Session-Id` | `sess_abc123` | Groups turns into a session — conversation continuity |

Missing headers = no memory mode for that request (falls through to standard proxy).

---

## Deployment Architecture

### Current: Python Shim (Flask Proxy)

The emulated memory subsystem runs inside the existing `vitriol_shim.py` Flask proxy:

```
OpenCode → VITRIOL Shim (port 5010) → llama-server (port 8279)
               │
               ├── Memory intercept (pre-request)
               │    ├── Read X-Project-Id / X-Session-Id headers
               │    ├── Query SQLite DB for relevant context
               │    ├── Compact & inject into system message
               │    └── Forward enriched prompt to llama-server
               │
               └── Memory storage (post-response)
                    ├── Store user + assistant episodes
                    ├── Update Hebbian edge weights
                    └── Signal consolidation thread
```

**Enabled via**: `VITRIOL_MEMORY_MODE=on` environment variable on the shim.

**Pros**:
- Zero modifications to llama.cpp or OpenCode
- Drop-in — works with any OpenAI-compatible client
- Fast iteration (Python, no rebuild needed)
- Consolidation thread runs in-process

**Cons**:
- Python GIL limits throughput under heavy load
- Flask adds ~1–3ms per-request overhead
- Not suitable for sub-millisecond latency requirements

### Future: Rust Native Daemon

When Python overhead becomes a bottleneck, the memory system should be extracted
into a standalone Rust binary:

```
OpenCode → Rust Memory Daemon (port 5010) → llama-server (port 8279)
               │
               ├── Async I/O (tokio) — handles 1000s conn/s
               ├── SQLite via rusqlite — zero-copy reads
               ├── tree-sitter — native AST parsing (Phase 3)
               ├── FAISS — GPU-accelerated vector search (Phase 2)
               └── Consolidation — tokio::spawn background task
```

**Why Rust wins here**:
- No GC pauses — critical for predictable latency
- True parallelism — each request gets its own thread, no GIL
- `rusqlite` is ~5x faster than `sqlite3` Python bindings
- `tokio` provides zero-cost async for the intercept-forward-response loop
- `tree-sitter` Rust bindings are first-class (official); Python bindings are wrappers

**Migration path**:
1. Keep Python shim for Phase 1 (MVP, keyword retrieval)
2. Port `memory/db.py` → Rust `rusqlite` module in Phase 2
3. Port `memory/retrieval.py` → Rust with `tokenizers` crate in Phase 2
4. Port `memory/hebbian.py` + `consolidate.py` → Rust `tokio` tasks in Phase 3
5. Replace `vitriol_shim.py` entirely — same API surface, drop-in replacement

### Alternative: Built into llama-server

Long-term, the memory intercept could be integrated directly into `llama.cpp`'s
`llama-server` as a request middleware:

```
OpenCode → llama-server (port 8279) — with memory middleware built in
               │
               └── Built-in memory hooks (C++ / CUDA)
                    ├── Intercept at HTTP handler level
                    ├── Query memory DB via C SQLite API
                    ├── Inject context into llama_context
                    └── Store episodes via async writeback
```

**This is not recommended** because:
- Tightly couples memory logic to inference engine
- Harder to maintain (llama.cpp changes frequently)
- C++ string handling for context injection is error-prone
- Better to keep a clean separation of concerns

**The Python Shim is the right call for now.** It proves the architecture,
the scoring functions, and the Hebbian update cycle. Rust migration happens
when the data volume demands it.

---

## File Layout

```
libvitriol/
├── __init__.py                          # Package init
├── client.py                            # Unix socket client
├── types.py                             # Data types
├── vector_store.py                      # FAISS wrapper
├── vitriol_shim.py                      # Flask proxy (entry point)
│
└── memory/                              # Emulated memory subsystem
    ├── __init__.py
    ├── db.py                            # SQLite schema + CRUD
    ├── scorer.py                        # Scoring function
    ├── retrieval.py                     # Keyword + cascade retrieval
    ├── compact.py                        # Token-budgeted injection
    ├── hebbian.py                        # Weight updates
    └── consolidate.py                    # Background summarization
```

---

## Configuration Reference

All settings are environment variables or `.vitriol/config.toml` entries.

### Memory Mode Toggle

| Variable | Values | Default | Description |
|----------|--------|---------|-------------|
| `VITRIOL_MEMORY_MODE` | `on` | `off` | Enable emulated memory system |
| `VITRIOL_MEMORY_DIR` | path | `~/.vitriol` | Root directory for state DBs |

### Retrieval Tuning

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMORY_TOP_K` | 5 | Max candidates to retrieve |
| `MEMORY_CASCADE_DEPTH` | 1 | Edge traversal depth (0 = no cascade) |
| `MEMORY_RELEVANCE_WEIGHT` | 0.4 | Score weight for keyword/text relevance |
| `MEMORY_RECENCY_WEIGHT` | 0.35 | Score weight for recency |
| `MEMORY_HEBBIAN_WEIGHT` | 0.15 | Score weight for edge weights |
| `MEMORY_STRENGTH_WEIGHT` | 0.10 | Score weight for node strength |

### Context Budget

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMORY_ACTIVE_BUDGET` | 4000 | Max tokens for injected memory context |
| `MEMORY_SESSION_KEEP` | 2 | Recent turns always included in budget |

### Consolidation

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMORY_CONSOLIDATE_EVERY` | 50 | Episodes between consolidation runs |
| `MEMORY_IDLE_SECONDS` | 60 | Idle time before consolidation triggers |
| `MEMORY_RETENTION_DAYS` | 30 | Delete unconsolidated episodes after |
| `MEMORY_NODE_DECAY` | 0.95 | Daily strength multiplier for nodes |

---

*Last updated: 2026-05-17 19:30 CEST*
*See also: `OPTIMIZATION_PLAN.md` (master roadmap), `CONTEXT_OFFLOADING_STRATEGIES.md`*
