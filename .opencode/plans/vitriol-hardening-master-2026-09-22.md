# VITRIOL Hardening Master Plan

Date: 2026-09-22 10:30
Status: IN PROGRESS
Owner: randozart (executed by agent session)

## Origin

User: "write a plan for the whole lot. Everything described. Everything that
would help make VITRIOL better. Write the plan, then begin work, however long
it takes, and document as you go."

Drivers:
1. 2026-09-22 audit of saas-home/Qwen3.8-27B-NV-16GB found GSQ-RCO IQ3_S-mtp
   is a strict win (+50% decode, flat depth, equal accuracy) — validated at
   reduced-ctx; needs blessed-geometry cert.
2. lull-kv investigation exposed: the current `main` engine has ZERO
   VITRIOL inference features (sparse/probe, per-device KV quant, tree-sitter,
   rectification). AGENTS.md claims they exist in main — **doc drift**. The
   runtime sparse is a silent no-op; the launcher guard is defeated by a
   glob-vs-symlink mismatch.
3. Multi-hop stress test is non-discriminative (test artifact).
4. lull worktree holds bounded-metrics + other features never ported.

## Goal

Make VITRIOL's daily-driver engine truthful to its config/fingerprint,
restore the sparse-KV preservation contract, adopt GSQ-RCO as the blessed
model, fix test-suite discriminativeness, correct doc drift. Each stage
committed + reported.

## Stages

### A. Model + config (no engine code)

- A1 GSQ-RCO full-262K ub=128 blessed-geometry cert:
  `c=262144 ts=24,24 ub=128 kv=q4_0/q4_0 fa=on spec=mtp:1 ckpts=4 cram=2048`
  ladder 8k/32k/64k/128k/200k/245k filled + accuracy arms (24k depth,
  no-thinking) + baseline Q3_K_M same geometry for parity.
- A2 config bless: path → GSQ-RCO, fingerprint diff vs previous blessed,
  unit restart, /health + fingerprint verify.

### B. Engine truthfulness + feature restore (code)

- B1 Launcher guard bug: `kv_sparse_guard` globs `libllama.so*` and passes on
  dead lull-era libs (.0.0.133/.134) while the runtime symlink
  `libllama.so.0 -> libllama.so.0.4.0` has no sparse. Fix: readlink-resolve
  the actual linked soname target and probe THAT.
- B2 Port lull-kv sparse → main: `vitriol-kv-probe.{cpp,h}` +
  `evict_sparse` + eager floor + `kv_ejected_total` counter + sink
  preservation. Rebase onto main's iswa memory refactor. ~540 lines, 10
  files. Verify `VITRIOL_KV_SCORE: probe active`, cadence-16 eviction,
  `⤓ Nk` counter, sink preservation.
- B3 Bounded /metrics scrape (lull 9c568a7d9, 42 lines): queue-wait starved
  telemetry under generation.
- B4 Per-device KV quant overrides (`VITRIOL_KV_QUANT[_K|_V]_GPU<d>`) — for
  duo-393K memory budget asymmetry.
- B5 Evaluate lull-only features for port: tree-sitter AST compaction,
  rectification hook, tool-reminder injection, config-profiles, split
  profiler/watermark. Port those still valuable on dual-3060.

### C. Tests + docs

- C1 Multi-hop stress test: constrained-reasoning variant (final-answer-only
  output) so it discriminates quant quality.
- C2 AGENTS.md doc drift: correct claims of features "in main" that are not;
  note lull-only status accurately.

### D. Integration

- D1 Full rebuild (sm_86 only — both cards Ampere), unit restart,
  fingerprint truthfulness verify (sparse now REAL), full blessed cert on
  GSQ-RCO, EXPERIMENT_LOG + report.

## Sequence

A1 → A2 → B1 → B2 → B3 → B4 → B5 → C1 → C2 → D1. B-stages each get a commit
per AGENTS "commit after every stage".

## Constraints (invariants)

- Residency rule: mode=off (model fits).
- OOM discipline: `--ctx-checkpoints >= 2`, `--cache-ram` sane; never
  `--ctx-checkpoints 0` / `--cache-ram 0`.
- Flag provenance: every launch fingerprint; every result embeds it.
- Never bare `serve` while unit present+active.
- Foreign GPU users (Bachi full64_stream loop, lares-llama unit) tolerated;
  cert geometry adjusts (reduced ctx if needed, stated).
- Inner fork untouched except deliberate ports; builds target sm_86 (both
  cards are RTX 3060 Ampere).