# VITRIOL Current Architecture (Generation 3 — llama.cpp fork + Spagyr + Copula)

> Date: 2026-08-06. This documents the current system as built in the 2026-08-06
> session. It supersedes the earlier kernel-module DMA architecture docs
> (`ARCHITECTURE.md`, `COOPERATIVE_DMA.md`), which describe the legacy NVMe→VRAM DMA
> design. See also: `copula.md`, `hermetis.md`, `spagyric-autotuner.md`,
> `spagyric-profile-schema.md`, `CHANGELOG_2026-08-06.md`, `BUGS.md`.

## 1. Overview — four layers

```
┌───────────────────────────────────────────────────────────────────┐
│ OpenCode — Copula Hermetis plugin (global)                        │
│   ingest: chat.message / message.part.updated / tool.execute.after│
│   retrieve: memory_search tool + per-turn auto-injection          │
│   provider: gen llama-server :8279/v1                             │
├───────────────────────────────────────────────────────────────────┤
│ Copula (the bond)                                                 │
│   Hermetis server :7980  (/hermetis/store|search|context|repo_map…)│
│   embed server :4779 (small embedding GGUF, CPU)                 │
├───────────────────────────────────────────────────────────────────┤
│ Hermetis (memory brain) — libvitriol/hermetis/                    │
│   SQLite per-project · retrieval · scorer · repomap · embed ·     │
│   node versioning (git_rev supersede) · rolling-window context    │
├───────────────────────────────────────────────────────────────────┤
│ VITRIOL runtime — llama.cpp fork                                  │
│   gen llama-server · TQ1_0/MoE · KV offload · profiles            │
│ Spagyr → renamed Spagyric — autotuner → [spagyric] profiles       │
└───────────────────────────────────────────────────────────────────┘
```

## 2. The two GPU servers

| server | port | model | role |
|---|---|---|---|
| gen | 8279 | Mellum2-12B Q4_K_M (or DeepSeek-Coder-V2-Lite) | generation backend for OpenCode |
| hermetis | 7980 | — (sqlite) | Hermetis memory RAG facade |
| embed | 4779 | bge-small-en-v1.5 Q8_0 | semantic embeddings for Hermetis |

> **Port scheme — Tria Prima (2026-08-07).** The three services map to the alchemical
> principles; each port encodes an atomic transmutation `<from><to>` (element numbers):
> gen=**Sulfur** 82→79 (Pb→Au, the Opus), hermetis=**Mercury** 79→80 (Au→Hg), embed=**Salt**
> 47→79 (Ag→Au). Zero source-of-truth in `scripts/vitriol-ports.sh`; env-overridable
> (`VITRIOL_GEN_PORT`/`VITRIOL_HERM_PORT`/`VITRIOL_EMBED_PORT`). Intentionally avoids the
> common dev port 8080.

Both are `llama-server` from the VITRIOL fork. Verified coexisting at **36–38 t/s gen
decode + full-speed embeddings** on a GTX 1070 Ti 8 GB (VRAM 7783/328 MiB) after a clean
rebuild. The generation server needs `cap_ipc_lock` (via `sudo vitriol setup`) only for
page-locked stream mode; VRAM-fit native models run without it.

## 3. VITRIOL runtime (llama.cpp fork)

- Fork base: upstream llama.cpp at `277ff5fff` (~#20920) + ~30 fork commits (TQ3/ternary
  kernels, MoE streaming, KV offload, buffer types, Mellum arch, Chimera dual-backend).
- **TQ1_0 / ternary** — natively supported (`ggml-quants.c`); verified against the
  dense BitNet 2B substrate.
- **KV cache offload** (`VITRIOL_KV_MODE=offload`) — host-RAM KV (Layer 1a) via
  `ggml_backend_dev_host_buffer_type`; CPU-layer fallback fixed (see BUGS.md). Measured:
  empty-context decode 18–21 t/s, used-context collapses (8K→7.7, 29K→5.1 t/s); VRAM
  path wins up to ~32K.
- **Setup**: `sudo vitriol setup` sets `cap_ipc_lock` + fixes RUNPATH (order fixed in
  this session).

### Gen model native KV/SWA profile (Mellum2-Claude-Thinking)

The agent model ships its own attention-level windowing tricks (verified from the GGUF
metadata + load log):

- **GQA**: `head_count_kv = 4` (vs 32 heads) → KV is 8× smaller than MHA.
- **Native SWA**: `sliding_window = 1024`, per-layer pattern over 28 layers (SWA on ~3/4
  of them) → those layers keep **only the last 1024 tokens** of KV, bounded regardless of
  context. Only ~7 full-attention layers carry the true context KV.
- **yarn scaling** factor 16 (orig 8192 → **131072** native context); `freq_base_swa` =
  500000 (separate rope base for SWA layers); head dim 128.

So total KV at 32K context is tiny — "context is rarely the limiter." Two layered windows:
the model's native SWA (bounded KV on most layers) and VITRIOL's server-side
`--context-shift` (rolls the full window). The shift is cheap because it only moves the
~7 full-attention layers' KV; the SWA layers self-bound.

## 4. Spagyric — the hardware autotuner

- **Origin**: the "weights-as-code" thesis (R2-FOLD) was measured **refuted** — bit-exact
  but **92.8× packed bytes** (code image 346 MB/tensor vs 3.73 MB packed; whole model
  ~73 GB), un-cacheable on 2 MB L2. That is the "package too large" wall.
- **Amortization discovered**: dense TQ1_0 GEMV batch amortization → **3.6× per-token**
  at batch R=16 (0.258 → 0.072 ms), knee at R≈16.
- **Decode-knob sweeps (real runtime)**:
  - `--parallel` is the decode throughput lever: DeepSeek 2.3× (57→134 t/s at p=8,
    168 at p=16/c=2048); Mellum 1.4× (p=4/c=32768 → 41.8, p=8/c=8192 → 53.9). VRAM is
    the ceiling (no compute knee on 8 GB).
  - `threads=4` fixed (t=8 catastrophic on this 4C/8T box: 2.24 t/s on Mellum).
  - `ubatch` not a decode lever (flat).
  - KV must stay in VRAM: `--no-kv-offload` → 15 t/s (CPU attention), q4_0 KV → 13.9 +
    crash — both refuted.
- **Profiles** (`profiles/<name>/config`): `deepseek`, `mellum2` with `[spagyric]`
  sections (parallel ceiling, box fingerprint, high-tput variants).
- Design: probe → sweep knobs → freeze profile. The `--spagyric-tune` flag is designed
  (not yet built); the sweep harness `libvitriol/spagyric_sweep.py` is the engine.

## 5. Copula — the VITRIOL↔OpenCode bond

The bond between OpenCode and VITRIOL's memory. Components:

- **Hermetis server** (`libvitriol/hermetis_server.py`, :7980): HTTP facade —
  `/hermetis/store`, `/node`, `/search`, `/context`, `/repo_map`, `/embed`, `/stats`.
- **Embed server** (:4779): llama-server with a small embedding GGUF (CPU `ngl=0`; the
  33M bge model is ~10-30 ms on CPU, and batch-scaled VRAM would starve the gen
  server), `/v1/embeddings`.
- **Copula Hermetis plugin** (`plugins/copula.ts` → `~/.config/opencode/plugins/`):
  ingest + `memory_search` tool + rolling window + file-change repo-map refresh.
- **`launch_copula.sh`**: start/stop Hermetis + embed. **`launch_vitriol_full.sh`**:
  setup(caps) + gen server + Copula in one command.

### Disabling Copula
- `COPULA_ENABLED=0` — plugin becomes a no-op.
- Delete `~/.config/opencode/plugins/copula.ts` — stop opencode loading it.
- `COPULA_AUTO_CONTEXT=0` — keep ingest, disable auto-injection.
- The plugin is non-blocking anyway (all requests fail silently when Hermetis is down).

## 6. Hermetis — the memory brain

**Engine** (`libvitriol/hermetis/`):
- `db.py` — per-project SQLite (`~/.vitriol/<project_id>/memory.db`): **episodes**
  (conversation turns + tool results), **knowledge_nodes** (**versioned**: `git_rev`,
  `superseded`, `superseded_by`), **edges**, sessions, embedding cache.
- `retrieval.py` — multi-hop retrieval (direct → edge cascade → score/rank);
  **current-version nodes preferred** (`superseded=0`), `include_history` opt-in;
  `context_block()` for rolling-window injection.
- `scorer.py` — relevance/recency/hebbian/strength scoring; semantic (GPU-GGUF or
  sentence-transformers fallback).
- `repomap.py` — Aider-style whole-repo map (regex symbols, import-graph in-degree
  rank, token budget).
- `compact.py` — formatting for injection. `consolidate.py`, `hebbian.py` — background
  consolidation + edge reinforcement.
- `embed.py` — embedding client (zero-guarded; **truncates inputs to the model's native
  512-token window**).

**Versioned-supersede (stale-data policy)**: nodes are never hard-discarded. A file
change (via `file.edited`/`file.watcher.updated` → plugin → single-file node refresh)
marks the old node `superseded` and lands a new current version; retrieval defaults to
current only, `include_history` opt-in.

## 7. Rolling window over a database

The context window is a *rolling window over Hermetis*: everything streams in,
compaction is lossless, and each turn the window is reassembled from what matters.

- **Ingest**: `chat.message` (full user turns) + `message.part.updated` (assistant) +
  `tool.execute.after` (tool results) + `session.idle` transcript sweep → Hermetis.
- **Lossless compaction**: `experimental.session.compacting` dumps the pre-compaction
  context to Hermetis as `[compaction capture]` — compaction can never lose anything
  the model saw.
- **Per-turn auto-injection**: on a new user message, retrieve `/hermetis/context`
  (budget-capped, recency+relevance) and inject labeled `[Hermetis context]` via
  `session.prompt({ noReply })`.
- **Toggles**: `COPULA_ENABLED` (on), `COPULA_AUTO_CONTEXT` (on),
  `COPULA_CONTEXT_BUDGET` (3000), `COPULA_CONTEXT_TOP_K` (5).

## 8. Configuration reference

| env | default | purpose |
|---|---|---|
| `COPULA_ENABLED` | on | master plugin toggle (`0` = no-op) |
| `COPULA_AUTO_CONTEXT` | on | per-turn auto-injection |
| `COPULA_CONTEXT_BUDGET` | 3000 | injected context token cap |
| `COPULA_CONTEXT_TOP_K` | 5 | retrieved items per injection |
| `COPULA_HERMETIS_URL` | http://127.0.0.1:7980 | Hermetis server |
| `COPULA_EMBED_URL` / `VITRIOL_EMBED_URL` | http://127.0.0.1:4779 | embed server |
| `VITRIOL_SEMANTIC_MODE` | off | enable semantic embeddings |
| `VITRIOL_KV_MODE` | — | `offload`/`sparse` KV placement |
| `VITRIOL_MEMORY_DIR` | ~/.vitriol | memory DB root |

## 9. How to run

```fish
# full stack: setup(caps) + gen :8279 + Hermetis :7980 + embed :4779
sudo /home/randozart/Desktop/Projects/VITRIOL/scripts/launch_vitriol_full.sh
# or memory-only:
./scripts/launch_copula.sh
# restart opencode (loads the global Copula Hermetis plugin); point its provider at
# http://127.0.0.1:8279/v1
# ops dashboard (Ratatui, builds on first use):
vitriol tui
```

## 10. vitriol-tui — the Ratatui ops dashboard

`vitriol-tui/` is a standalone Rust TUI (ratatui + crossterm + ureq) launched
by `vitriol tui`. Five tabs, themed "Vitriolum" (dark alchemical green + gold,
Alka Officina–derived, plan `2026-08-07-vitriol-tui.md`):

| tab | content |
|---|---|
| DASHBOARD | gen/hermetis/embed health, GPU gauges, decode-t/s sparkline |
| GPU | btop-style gauges (VRAM/util/temp/clocks/power) + process table |
| LOGS | live tails of gen/hermetis/embed logs, `[1/2/3]` source switch |
| CONTROLS | start/stop/restart, doctor, `vitriol setup`, Spagyric sweep |
| PROFILES | active-config form editor + profile list: load, select-for-start, save-as-new, overwrite, delete, sweep |
| HERMETIS | stats, recent stores (`GET /hermetis/recent`), search |

Profile/start semantics (2026-08-08): loading a profile just writes its config
into the active `~/.vitriol/config` (no relaunch). The PROFILES tab keeps a
**selected** profile (`t`) that Start/Restart apply as CLI flag overrides
(`--model/--ngl/--ctx/--threads/--parallel`, config file untouched). A successful
Spagyric sweep+save auto-selects its `<name>-swept` winner as the Start target.

Data comes from the HTTP endpoints + `nvidia-smi` + log tails; control shells
out to `scripts/launch_vitriol_full.sh` and `scripts/vitriol` (reuse, don't
reimplement). Decode t/s is parsed from the gen log's `eval time` lines (the
server `/health` only returns `{"status":"ok"}`). The default Hermetis project
id is the sanitized full cwd path, matching hermetis `_project_id`.

## 11. Pymander — the reference mind (P1: store + ingest)

Hermetis remembers *what happened* (episodic); **Pymander** is the static,
curated answer to *how we do a domain well*. Content is hand-authored **atomic
nodes** (small pieces of task-relevant knowledge). Plan
`2026-08-07-pymander-p1.md`, building on `2026-08-06-pymander-ascensus.md`.

Store: each domain is a distinct Hermetis memory root
`~/.vitriol/pymander/<domain>/memory.db`, reached via `hermetis.db` with
project_id `pymander/<domain>` — so node **versioning** (git_rev supersede),
**strength**, and the **embedding cache** all come free. Ingest turns markdown
(`## heading` → atomic node, `docs/pymander/*.md`) into nodes, embeds
best-effort (see §4 embedding; keyword falls back when the embed server is
down).

CLI: `vitriol pymander {list|ingest|nodes|search|select|active}` →
`libvitriol/pymander.py`. Per-project selection (which domains are active for a
project) persists to `~/.vitriol/pymander/selection.json`.

Out of scope so far (P2+): doctrine injection into the window, the `pymander`
opencode tool, Ascensus, and the promotion path.

### P2 — injection + tool (2026-08-07)

`/pymander/*` endpoints on the Hermetis server: `list`, `search`, `select`,
`context` (budgeted doctrine block across a project's selected domains).
Copula plugin (`plugins/copula.ts`) gains a `pymander_search` tool and injects
a `[Pymander doctrine]` block once per session on the first user message
(reuses the Hermetis auto-inject machinery; label added to the re-ingest skip
list). Doctrine content follows the project's `selection.json`.

## 12. Key measured facts (design constraints)

| fact | value |
|---|---|
| fast context window (VRAM KV) | ~32K |
| max context (KV offload, slow) | 131K (model native cap) |
| gen decode (Mellum, clean build) | 36–38 t/s |
| parallel lever | up to VRAM ceiling (p=8@c=4096 DeepSeek) |
| threads | 4 fixed (t=8 catastrophic) |
| weights-as-code (R2-FOLD) | refuted, 92.8× |
| dense batch amortization | 3.6× at R=16 |
| embedding model | bge-small-en-v1.5 Q8_0, 384-dim, CPU (native 512-token window; inputs truncated) |

## 13. Repository map (this session's files)

- `libvitriol/hermetis/` — memory engine (db, retrieval, scorer, compact, consolidate,
  hebbian, repomap, embed).
- `libvitriol/hermetis_server.py` — Hermetis HTTP facade (`/hermetis/recent` + `/pymander/*` added for TUI/Pymander).
- `libvitriol/spagyric_sweep.py` — Spagyr sweep harness.
- `libvitriol/pymander.py` (+ `pymander_test.py`) — Pymander reference-mind store/ingest (see §11).
- `vitriol-tui/` — standalone Ratatui ops dashboard (see §10).
- `plugins/copula.ts` — Copula Hermetis plugin (versioned; installed to opencode global).
- `profiles/{deepseek,mellum2}/config` — frozen `[spagyric]` profiles.
- `scripts/launch_copula.sh`, `scripts/launch_vitriol_full.sh`, `scripts/praetor_diff.py`,
  `scripts/build-llama-server.sh` (fixed).
- `.praetor.toml` — intent-comment check disabled (rule rejects valid Python comments).
- Docs: `copula.md`, `hermetis.md`, `spagyric-autotuner.md`, `spagyric-profile-schema.md`,
  `pymander/` (corpus), `provenance/pymander.md`, this file, `CHANGELOG_2026-08-06.md`,
  `BUGS.md`.

## 14. Provenance

VITRIOL and its forks are the user's own repos (AGENTS.md §2.2: freely borrowable).
Everything is original re-derivation or the user's own work. Aider's repo-map *idea*
(inspiration, not code) informed `repomap.py`. Measured boundaries (R2-FOLD refutation,
amortization, decode-knob results) are recorded in the plans listed in
`CHANGELOG_2026-08-06.md`.
