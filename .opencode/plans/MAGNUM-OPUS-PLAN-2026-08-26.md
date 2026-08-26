# MAGNUM OPUS — Robust Memory Architecture Plan

> **Date**: 2026-08-26 · **Status**: executing
> **Goal**: a self-verifying, robust, memory-unconstrained agent stack running
> on weak hardware (i7-3770, 16 GiB DDR3, 3060+1070 Ti).
>
> Doctrine: every dependency is optional, every store rebuildable from a lower
> tier, every failure degrades to silence, every fact carries provenance, and
> every guarantee is drilled — not assumed.

## 1. Failure model (what robustness means here)

| # | mode | defense |
|---|---|---|
| F1 | store outage | degradation ladders; stale-serving; per-provider isolation |
| F2 | split-brain | single cross-agent brain (Hermetis) + conflict surfacing |
| F3 | echo/feedback | StreamingContextScrubber (hermes) + ingest scrub (engine-side port) |
| F4 | compaction loss | provider `on_pre_compress` durable checkpoint (API v1) |
| F5 | poisoning | consolidation gating + provenance + conflict flags |
| F6 | staleness | recency/hebbian scoring verified; decay drill |
| F7 | recall miss | redundant read paths (files / graph-search / cosine / doctrine) |
| F8 | semantic drift | embedding-model registry: version-stamped vectors + reindex-on-mismatch |
| F9 | secrets ingress | `isSecretPath`-equivalent scrub at every ingest point |
| F10 | write races | single-writer services; SQLite WAL; atomic file renames |

## 2. Tiered epistemics

```
T0 CURATED DOCTRINE   Pymander markdown (human-authored, git-tracked)
                      ↕ hash-gated mirror with version-stamped embeddings
T1 CONSOLIDATED       Hermetis knowledge_nodes — machine-consolidated,
   KNOWLEDGE          hebbian-weighted, provenance-tagged, decays
T2 RAW EPISODES       append-only turns from opencode / hermes / ascensus
```

Recall = union of independent paths: builtin file notes · Hermetis multi-hop
search · vitriol_rag cosine · Pymander domain injection. Each path dies
independently.

## 3. Ownership matrix

| use case | owner |
|---|---|
| hermes private session notes | builtin MEMORY.md/USER.md toolset (stays) |
| opencode working memory | Hermetis via copula (status quo) |
| **cross-agent project knowledge** | **Hermetis** — both agents feed & recall |
| curated doctrine | Pymander (+ derived Mongo index) |
| ascensus dedup cache | Hermetis escalation episodes |
| euro ledger | ascensusd single-writer JSON |

## 4. Work breakdown

### Phase 0 — Groundwork
- 0.1 Read engine internals (`libvitriol/hermetis/{db,retrieval,scorer,
  consolidate}.py`, `hermetis_server.py`); verify decay semantics, schema,
  embedding storage. No hardening claims before this.
- 0.2 Revive Hermetis `:7980` as hardened user unit (`Restart=always`,
  memory cap, WAL check). It has been down silently — never again.
- 0.3 Baseline RAM/endpoint measurements for the final report.

### Phase 1 — ascensusd (:8283)
Single-writer escalation core: dedup (Hermetis search ≥0.6) → euro-budget
gate (worst-case estimate vs ASCENSUS_EUR_DAILY=1/MONTHLY=30) → Gemini call
→ usageMetadata actuals → store-back with `{agent:"opencode"|"hermes"}`
provenance. Secrets scrub on files. Degradation ladder: Hermetis down ⇒ skip
dedup but still enforce budget and mark uncached. copula.ts becomes thin POST.

### Phase 2 — hermes tie-in
- 2.1 `ascensus` skill: policy (mirror of AGENTS.md rules) + scripts wrapper.
- 2.2 `$HERMES_HOME/plugins/memory/hermetis/` MemoryProvider adapter, full
  surface: is_available/initialize/system_prompt_block/prefetch/sync_turn/
  **pre_compress checkpoint v1**/**get_tool_schemas+handle_tool_call**
  (`hermetis_search`, `hermetis_remember` verbs)/shutdown. Installed as user
  plugin — zero hermes-repo changes. Selection via `memory.provider: hermetis`
  stays an explicit operator choice; builtin remains default.

### Phase 3 — E2 engine hardening (libvitriol/hermetis, our code)
- 3.1 Provenance columns `{agent, session, ts}` on episodes/nodes
      (additive ALTER TABLE, backward-compatible reads).
- 3.2 Server-side secrets scrub at `/hermetis/store` + `/node`.
- 3.3 Conflict surfacing in retrieval: same-label divergent-content nodes are
      returned flagged `conflict_with`, never silent last-write.
- 3.4 Decay verification in `scorer.py`: prove recency actually demotes;
      tune only if broken; add decay test.
- 3.5 Embedding registry: `embedding_model` column + version stamp on write;
      retrieval skips/mismatch-marks foreign-model vectors.

### Phase 4 — E4 embedding consistency (Pymander mirror)
- Version-stamp vectors; reindex job gated on model mismatch; wired into
  pymander_sync + vitriol_rag stats.

### Phase 5 — E3 git-versioned snapshots
- `scripts/memory_snapshot.py`: knowledge_nodes → `docs/memory-snapshot/
  <project>/*.md` (frontmatter: label/strength/provenance/updated). systemd
  timer daily. Binary DB operational; markdown recoverable truth.

### Phase 6 — E5 chaos suite
- `scripts/chaos_drill.py --all`: kill each dependency mid-operation
  (mongod, vitriol-rag, hermetis, llama-server), assert contracts:
  - echo regression: injected block stored ⇒ scrubbed on re-store
  - split-brain reconciliation: same fact via two agents ⇒ one surfaced node
  - budget refusal under exhaustion; dedup hit costs €0
  - stale-serving during mongo outage; empty-not-error cold failure
- Exit code = pass/fail; runs pre-commit of any memory-layer change.

### Phase 7 — Docs
- `docs/MEMORY_OWNERSHIP.md` (matrix codified), OPERATIONS/GLOSSARY updates,
  execution report with measured RAM deltas and drill outputs.

## 5. Non-goals

- No migration of hermes builtin storage; no dual-write echo modes.
- No Atlas/vector-index infrastructure (client cosine correct at scale).
- No cloud embeddings (local bge-small only) for internal recall.
