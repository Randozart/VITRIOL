# Session Log — 2026-05-17

## Summary

Emulated Memory Architecture — design docs, 7-module Python memory subsystem, shim integration, and CLI config toggle with port-swap launch logic. 957-line CLI script, ~1,400 LoC memory package, 1,182 LoC design docs.

## Deliverables

| Artifact | Lines | Status |
|----------|-------|--------|
| `docs/OPTIMIZATION_PLAN.md` (V2) | 590 | ✅ 4-layer roadmap, 7 cited papers |
| `docs/EMULATED_MEMORY_ARCHITECTURE.md` | 592 | ✅ DB schema, scoring, cascading retrieval, Hebbian, compaction, sleep, deployment |
| `libvitriol/memory/` (7 modules) | ~1,400 | ✅ db, scorer, retrieval, compact, hebbian, consolidate, __init__ |
| `libvitriol/vitriol_shim.py` (memory toggle) | 763 | ✅ `VITRIOL_MEMORY_MODE=on` intercept loop, `/memory/stats`, `/memory/clear` |
| `scripts/vitriol` (TUI + port swap) | 957 | ✅ Memory Settings menu, `--memory-mode` flag, detach/foreground port swap |
| `~/.config/opencode/opencode.jsonc` | — | ✅ X-Project-Id, X-Session-Id custom headers |

## Key Decisions

- **Memory mode off by default.** Enable with `VITRIOL_MEMORY_MODE=on` or `vitriol serve --memory-mode on`.
- **Port swap (Option A).** llama-server on PORT-1 (8278), shim on PORT (8279) when memory mode is on. OpenCode config never changes.
- **Python Flask shim now.** Rust daemon later. Fastest path to functional prototype.
- **Episodic + semantic split.** Two table families for raw interaction context vs cross-session knowledge patterns.
- **Hebbian weight updates.** Post-response edge strengthening based on co-occurrence, not just recency.
- **Compaction on intercept (not generation).** Token budget enforced before forwarding to llama.cpp, never blocking token generation.

## Config Interface

```bash
# TUI
vitriol config → option 4) Memory Settings → toggle on/off

# CLI flag
vitriol serve --memory-mode on --detach
vitriol run --memory-mode on

# Env var
VITRIOL_MEMORY_MODE=on vitriol serve
```

## Architecture

```
OpenCode ──POST /v1/chat/completions──► vitriol_shim.py (port 8279)
                                              │
                                        1. Parse X-Project-Id
                                        2. Extract user intent
                                        3. Query memory DB (scoring + cascade)
                                        4. Inject retrieved context as system msg
                                        5. Forward prompt
                                              │
                                              ▼
                                        llama-server (port 8278)
                                        (8192-token context, never compacts)
```

## Next Steps

1. Test port swap end-to-end: `VITRIOL_MEMORY_MODE=on vitriol serve --detach`
2. Rust daemon (`vitriol-router`) — tokio + rusqlite + tree-sitter

---

## Session 2 (2026-05-17, 18:00) — Phase 1: Context Wins

**Three features implemented:** KV cache offload, sparse KV caching, frozen prompt caching.

### Changes

| File | Change |
|------|--------|
| `scripts/vitriol` | +70 lines: `--kv-mode`, `--frozen-prompt` CLI flags, defaults, config parse/write, TUI menu, env piping |
| `libvitriol/vitriol_shim.py` | `frozen_count` param in `rectify_context`, frozen prefix separation, hash-based change detection |
| `llama.cpp/src/llama-kv-cache.cpp` | `VITRIOL_KV_MODE=offload` uses host buffer type; `evict_sparse()` for position-based eviction; sparse hook in `prepare()` |
| `llama.cpp/src/llama-kv-cache.h` | `evict_sparse()` declaration |
| `llama.cpp/src/llama-kv-cells.h` | `score` vector + `score_get/set/add` accessors + reset in rm/seq_rm/seq_keep |
| `llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu` | Remove `integrated &&` guard on `cuda_host_buffer` in `supports_buft` |

### Config Interface

```
--kv-mode standard | offload | sparse    (env: VITRIOL_KV_MODE)
--frozen-prompt on | off                 (env: VITRIOL_FROZEN_PROMPT)
```

TUI accessible via: vitriol config → option 4) Context & Memory Settings

### Build Required

The C++ changes to llama.cpp require a rebuild:
```bash
cd llama.cpp && cmake --build build -j$(nproc)
```
