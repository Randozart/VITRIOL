# Closed-Loop Local Cognitive Architecture — calibrated design note

Date: 2026-08-06.

## The design

A closed loop of three systems, mapping onto what is actually built/being built:

```
model (reasoning)  --decides retrieval-->  Hermetis (out-of-core store)
    ^                                            |
    |              [Hermetis context]            v
    +---------- selective injection <--- active sliding window (VITRIOL)
```

1. **Fixed sliding context window** — `--context-shift` on the VITRIOL server rolls the
   window (drains the front past `--ctx 32768`, keeps system + recent). Bounded KV,
   no unbounded growth, no opencode compaction (its `limit.context` = 131072 so the
   threshold is never reached).
2. **Hermetis** — long-term out-of-core memory (episodes + versioned repo-map nodes).
   Selectively injects only required context (`/hermetis/context`: `min_score` floor +
   `is_new_topic` gate + budget 1500).
3. **Reasoning model** — decides *when/what* to retrieve via the `memory_search` tool
   (the `<think>` block is hidden `reasoning_content`; the tool-call is the visible
   mechanism).

## Calibrated claims (what is true vs oversold)

**True:**
- Fixed KV window removes unbounded KV growth / compaction-driven re-allocation.
- Selective injection avoids relying on long-context attention (no "attention decay").
- Reasoning models make the retrieval-tool decision more reliable — the exact weakness
  (agentic unreliability) we are targeting with the Mellum2-Thinking swap.

**Oversold / not accurate:**
- "Zero fragmentation, fully static, no runtime heap allocations" — only the KV is
  fixed-size; llama.cpp still allocates compute buffers / cudaMalloc at runtime.
- "Hermetis stores ASTs" — it stores episodes + repo-map **symbol signatures** (regex),
  not full ASTs. Tree-sitter AST indexing is an upgrade path, not built.
- **"Spagyric compiles models into hardware-native instruction layouts" is the REFUTED
  thesis (R2-FOLD, 92.8x, `.opencode/plans/2026-08-06-spagyric-shader-test.md` §14).**
  Spagyric is the hardware autotuner (sweeps -> [spagyric] profiles), not a
  weights-to-instructions compiler. Keep the two distinct.
- "Chimera zero-latency CUDA+Vulkan" — marketing; it is the dual-backend path.

## Why this maps cleanly onto the built system

| design element | built artifact |
| --- | --- |
| sliding window drain | VITRIOL `--context-shift`, `--parallel 1`, `--ctx 32768` (launch script) |
| never-compact | opencode `limit.context: 131072` (opencode.jsonc) |
| out-of-core store | Hermetis (SQLite per-project, episodes + versioned nodes) |
| selective re-inject | `/hermetis/context` (`min_score`/`is_new_topic`/budget) + plugin gates |
| lossless capture | plugin ingest + `session.compacting` hook |
| retrieval decision | `memory_search` tool (explicit) + auto-inject (proactive, gated) |

## Status

- Code: committed (`62401a9` window work; plugin synced). Verification pending on GPU
  (avatar capture holds it).
- Reasoning-model swap: in progress (Mellum2-Claude Thinking downloading) — see
  `.opencode/plans/2026-08-06-mellum-thinking-agent-swap.md`.
