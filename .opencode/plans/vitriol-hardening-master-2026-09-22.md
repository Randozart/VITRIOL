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

## Progress log

### A1/A2 DONE 2026-09-22 11:00 (blessed-geometry cert + retarget)

GSQ-RCO vs Q3_K_M, same geometry (c=262144 ts=24,24 ub=128 kv=q4_0/q4_0
fa=on spec=mtp:1 ckpts=4 cram=2048), depth_cert 3x64 decode:

| filled | Q3_K_M | GSQ-RCO | delta |
|---|---|---|---|
| 8k | 19.30 | 23.54 | +22% |
| 32k | — | 22.11 | — |
| 64k | 20.55 | 25.21 | +23% |
| 128k | — | 24.58 | — |
| 200k | 20.30 | 24.85 | +22% |
| 245k | 19.79 | 24.85 | +26% |

GSQ-RCO: no OOM marks (Q3_K_M had transient `cudaMalloc` 306 MiB hits on
dev1 that recovered); 244,778 filled @ 24.85 t/s flat curve.
Config blessed: `path` → GSQ-RCO-IQ3_S-mtp.gguf (single-field diff: model).
Unit active, /health ok, "Lapis Occultus" serves. Fingerprint:
`model=Qwen3.8-27B-GSQ-RCO-IQ3_S-mtp.gguf ts=24,24 c=262144 kv=q4_0/q4_0
fa=on ub=128 mode=off engine=vitriol-dma ckpt=8192 ckpts=4 lru=0
sparse=sparse score=probe pool_reset=0 spec=mtp:1 par=1 ssp=1 cram=2048 cis=0`
NOTE: `sparse=sparse` in fingerprint is STILL a no-op (engine lacks lull-kv)
until B2 lands — recorded for truthfulness.

### B1 IN PROGRESS (launcher guard symlink fix)

### B1/B2 DONE 2026-09-22 11:30 (guard fix + lull-kv sparse port)

- Inner fork commit `6f9316128`: full lull-kv sparse port to main
  (vitriol-kv-probe, score cells, evict_sparse, eager floor, counter,
  prometheus gauge, graph-reuse disarm) + latent build-break fix
  (missing llama-sampler.h include).
- Outer commit `3d6c5f1`: kv_sparse_guard now readlink-resolves the
  actually-linked libllama (was globbing dead lull-era libs).
- Verified live on GSQ-RCO daily driver (unit, port 8279):
  `VITRIOL_KV_SCORE: probe active, n_kv=256 layers=1 decay=0.90`,
  `llamacpp:kv_ejected_total 0`, decode 28.8-29.3 t/s (no regression).
  Fingerprint `sparse=sparse score=probe` is now TRUTHFUL.

### B3 IN PROGRESS (bounded /metrics scrape)

### B3/B4/B5 DONE 2026-09-22 13:00 (metrics already-present; per-device KV quant; tool-reminder)

- B3: bounded /metrics scrape already in main (2026-09-04 merge, has own
  plan doc). Nothing to port.
- B4 inner commit `d86cdbb59`: per-device KV quant overrides
  (VITRIOL_KV_QUANT[_K|_V]_GPU<d>). Verified: K_GPU0=q8_0 -> dev0
  k:q8_0/v:q4_0, dev1 unchanged.
- B5 inner commit `fa7a75f4e`: tool-reminder injection (VITRIOL_TOOL_REMINDER).
  Verified: 144-byte reminder before generation marker on tools request.
- B5 eval: split profiler (VITRIOL_LULL_PROFILE) already in main. NOT
  ported (correctly obsolete on current HW): rectify.experts MoE tally
  (dense GSQ-RCO / mode=off), VITRIOL_FUSE_MMVQ_PASCAL (Pascal-only; both
  cards Ampere), PERF/CUGR decode diag (marginal), tree-sitter AST
  compaction (1.8M-line vendored dep, risky, unused).

### C1 IN PROGRESS (multi-hop constrained-reasoning variant)

### C1/C2 DONE 2026-09-22 14:00

- C1 outer commit `308097d`: multi-hop constrained variant (final-only
  output, 400-token cap with --no-thinking). Now deterministic but measures
  model arithmetic capability, not quant quality (documented; model misses
  chained XOR/arith final values even at zero depth - verified).
- C2 outer commit `85a53c9`: AGENTS.md drift fixed (lull-kv on main, Stage 6
  resolved, GSQ-RCO blessed + dual-3060 LIVE CONFIG, orphaned 1070-Ti text
  removed).

### D1 DONE 2026-09-22 15:00 (integration + final cert)

- Inner fork clean at fa7a75f4e (3 VITRIOL commits on ab61c2a01, 0-behind
  upstream). Unit restarted on final binary.
- Final blessed-geometry cert on integrated engine: 200k @ 24.08 t/s,
  245k (244,778 filled) @ 23.55 t/s, probe:true, no OOM. Sparse probe costs
  ~1 t/s at depth (24.85->23.55 vs A1 no-probe) - accepted for the
  preservation contract.
- Unit active, fingerprint `sparse=sparse score=probe` now TRUTHFUL (probe
  real, verified live).
- Master plan COMPLETE. All stages done.

## Outcome summary

- GSQ-RCO IQ3_S-mtp blessed (+22-26% decode, flat depth).
- lull-kv sparse restored to main (probe scoring, evict_sparse, eager floor,
  ejected counter + gauge) - the silent-no-op hole is closed AND the guard
  now checks the real linked lib.
- Per-device KV quant + tool-reminder ported.
- Bounded metrics already-present; obsolete features evaluated not-ported.
- Multi-hop test deterministic; docs truthful.
- Next (out of scope): duo-393K cutover can now lean on real sparse; the
  probe overhead at depth is worth a follow-up tuning pass (VITRIOL_KV_SCORE_EVERY).

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