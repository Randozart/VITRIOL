# VITRIOL Work Ledger — 2026-08-06

Complete record of the 2026-08-06 session: 39 VITRIOL + 27 bitshaper-ai commits.
This is a *historical record* — reference it, never edit retroactively. Findings and
root causes also live in `BUGS.md`; plans in `.opencode/plans/`.

## Arc

1. Weights-as-code thesis → measured refutation → amortization discovery.
2. Spagyr → **Spagyric**: the VITRIOL hardware autotuner (sweeps + frozen profiles).
3. Layer 1a KV-offload investigation (2 bug fixes).
4. **Copula**: the VITRIOL↔OpenCode bond; **Hermetis**: the memory system.
5. Rolling window over a database (lossless compaction + auto-injection).
6. Full-stack launch scripts.

## Phase 0 — baselines + autotuner concept

- Reproduced clean correctness-gated baselines on the rebuilt fork: DeepSeek-Coder-V2-Lite
  IQ2_M **58.1–58.3 t/s**, Mellum2-12B Q4_K_M **30.9–34.3 t/s**, both PASS (merge sort).
  The prior "illegible output" concern did not reproduce.
- Fixed the VITRIOL pre-commit hook: whole-repo Praetor → staged-files → **diff-aware**
  (`scripts/praetor_diff.py`): only NEW diagnostics relative to the staged version gate
  commits (AGENTS.md §8).
- Disabled Praetor's intent-comment check (`.praetor.toml`): the rule rejects valid
  Python `#` comments (verified with a minimal test file).
- **Spagyric reframe** (from Judith van Stegeren consultation + TRT-LLM precedent):
  not a weights-as-code compiler — a **VITRIOL hardware autotuner**. Renamed Spagyr →
  **Spagyric** (git mv, history preserved).

## Refutation + discovery (bitshaper-ai engine)

- **R2-FOLD (weights-as-code) refuted**: `crates/engine/examples/blkfold.rs` — bit-exact
  (PARITY PASS, max_abs_diff=0) but **92.8× packed bytes** (346 MB/tensor vs 3.73 MB;
  whole model ~73 GB), un-cacheable on Pascal's 2 MB L2. The "package too large" wall,
  measured.
- **Batch amortization (dense TQ1_0)**: 3.6× per-token at batch R=16 (0.258 → 0.072 ms),
  knee at R≈16, parity bit-exact. Carried into the runtime sweeps.

## Decode-knob sweeps (real runtime)

- `libvitriol/spagyric_sweep.py` (mode A single-request, mode B concurrent):
  - **DeepSeek**: ubatch flat, threads flat (GPU-bound), **parallel 2/4/8 → 78.5/87.9/
    134.1 t/s (2.3×)**; p=16@c=2048 → 168 t/s (no compute knee; VRAM is the cap).
  - **Mellum**: ubatch flat, **t=8 catastrophic (2.24 t/s)**, parallel 2/4 → 37.2/41.8
    (1.4×); p=8@c=8192 → 53.9.
  - **KV/context levers**: `--no-kv-offload` refuted (15 t/s, CPU attention);
    q4_0 KV refuted (13.9 + crash). KV stays in VRAM; context = `parallel × ctx` budget.
- Frozen `[spagyric]` profiles: `profiles/deepseek` (p=8@c=4096 → 134 t/s) and
  `mellum2` (p=4@c=32768 → 41.8, high-tput p=8@c=8192 → 53.9).

## Layer 1a — KV cache offload

- **Bug fixed**: `VITRIOL_KV_MODE=offload` aborted on CPU-placed layers
  (`get_host_buffer_type` NULL on the CPU backend → `buft_is_host(NULL)` assert).
  Submodule `85d01eda8`.
- **Bug fixed**: `vitriol setup` ran `fix_rpath` (patchelf) after `setcap`, clearing the
  capability. Reordered (`a583047`).
- Measured: empty-context decode 18–21 t/s across 32–131K allocated (vs 30–34 VRAM);
  used-context collapses (8K→7.7, 29K→5.1 t/s); 131K is the model native cap; 200K
  impossible.

## Copula + Hermetis

- **P1**: Hermetis server (`libvitriol/hermetis_server.py`) — `/hermetis/store|node|
  search|stats|health`; `db.store_node`. **Bug fixed**: uncommitted edge write left an
  open write transaction → 3rd sequential store stalled ~5 s under threaded Flask
  (busy_timeout). Param bundling (`EdgeSpec` + `meta`) per AGENTS.md §5.3.
- **P2**: GPU-GGUF embedding provider (`hermetis/embed.py`, `/hermetis/embed`, semantic
  store-warm). **Investigation**: the "fork BERT zero-embedding bug" does **NOT
  reproduce in the current source** — it was a stale P2-era binary. GGUF-GPU embeddings
  verified working (norm 1.0, all inputs). Backlog closed. sentence-transformers stays
  as CPU fallback; zero-guard stays defensive.
- **P3**: node versioning (`git_rev` supersede, current-only retrieval, `include_history`
  opt-in) + Aider-style repo map (`repomap.py`, `/hermetis/repo_map`) + plugin file-change
  trigger. P5 validated: file edit → old node superseded, new current lands.
- **P4**: Copula Hermetis plugin (`plugins/copula.ts`) — ingest + `memory_search` tool.
- **Bug fixed**: `get_edge_targets` UNION column mismatch after the P3.1 migration
  (episodes `e.*` UNION nodes `n.*` diverged).
- Renames: memory system = **Hermetis**; the bond = **Copula**; the plugin =
  **Copula Hermetis**.

## Rolling window over a database

- **C**: `/hermetis/context` — budget-capped, recency+relevance context block
  (`4c7312d`).
- **A**: plugin `chat.message` full capture + `experimental.session.compacting`
  lossless dump (`a99052d`).
- **B**: per-turn auto-injection (labeled `[Hermetis context]`, `session.prompt noReply`,
  `COPULA_AUTO_CONTEXT` toggle) (`a99052d`).
- Validated (Hermetis side): ingest → context block (capped) → compaction capture →
  retrieval finds original + capture (0.863) — nothing lost.
- `COPULA_ENABLED` master toggle (`692700a`).
- API verified on installed opencode **1.15.13**: `session.compacted`, `session.prompt`
  `noReply`, `session.messages`, `event`, `tool.execute.after`,
  `experimental.session.compacting`, `chat.message`.

## Launches + build

- `scripts/launch_copula.sh` — start/stop Hermetis + embed (`541a83e`).
- `scripts/launch_vitriol_full.sh` — setup(caps) + gen server + Copula (`cfa0ef9`).
- **Bug fixed**: `build-llama-server.sh` fails on clean builds — vendored cpp-httplib
  linked into `libllama-common.so` without `-fPIC`. Added
  `-DCMAKE_POSITION_INDEPENDENT_CODE=ON` (`cfa0ef9`).
- **Stale-build findings**: the P2-era binary produced zero BERT embeddings and the
  full-launch binary showed 8–15 t/s decode — BOTH were stale-build artifacts. A clean
  rebuild yields correct embeddings and **37–38 t/s** (better than baseline).

## Model/assets

- Downloaded: `bitnet-2b-tq1_0.gguf` (dense ternary substrate, verified),
  `nomic-embed-text-v1.5` (Q8_0/F16), `bge-small-en-v1.5-q8_0.gguf` (embedder).
- Finding: `qwen3.6-35b-a3b-instruct-TQ1_0.gguf` produces garbage in every mode (stream
  + CPU) — a bad model file, not the fork. Stream/VITRIOL-knob sweep deferred behind a
  known-good file.

## Commits (VITRIOL, 39; bitshaper-ai, 27)

See `git log --since="2026-08-06 00:00"` in each repo. Key milestones: `6fd83b2`
(Spagyric concept), `b6aa158` (profiles), `63c3e5a` (P1 + edge-write fix),
`f1e62ae`/`5f6c149` (P2 + investigation), `a2b178c` (P3), `8033d9e` (P4),
`4c7312d`/`a99052d`/`2ea38db` (rolling window), `541a83e`/`cfa0ef9` (launches),
`692700a` (COPULA_ENABLED).
