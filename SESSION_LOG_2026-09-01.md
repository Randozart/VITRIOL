# Session Log 2026-09-01 — Mining Program Launch + E1 Upstream Sync

Anchored summary (top, per protocol; appended as session progresses).

## Summary

- **Goal**: convert the 2026-09-01 mining pass (upstream llama.cpp, OurobourOS,
  bitshaper-ai, kimi-k3-in-c) into a scientific experiment program and execute
  it. Master plan:
  `.opencode/plans/mining-experiment-master-plan-2026-09-01.md` (E1-E7,
  protocol P-1...P-10, frozen baselines).
- **Pre-work**: user request - second Officina slot disabled.
  `~/.vitriol/config` `parallel = 2 -> 1`; systemd unit description
  dual->single-slot; server restarted; single slot now owns full 81920 window.
- **E1 EXECUTED** (upstream sync): merged upstream `be789c344` into inner
  `main` (`04a4f5f12`, zero conflicts), built `build-ku2/` (61;86, nvcc 12.9 +
  g++-14 host, ccache wired), full A/B vs frozen baselines.
- **E1 Results**: H0 PASS (no regressions; +0.4-1.3% fa-on shallow drift).
  H1a NULL (deep-ctx gain is multi-seq-conditioned; we run parallel=1).
  H1b not reproducible (lineage never slow). H1d no end-to-end delta (mma
  path, sm_61 N/A). E6 lazy-mode neutral/adopted. Report:
  `.opencode/plans/e1-upstream-sync-2026-09-01.md`. EXPERIMENT_LOG updated.
- **E3 EXECUTED** (oracle + parity ladder): new `tools/vitriol-oracle`
  capture tool + `diff.py`; all gates green (determinism byte-exact,
  perturbation caught, cross-backend divergence characterized as quantized
  matmul rounding; greedy-equal). Ladder adopted in `llama.cpp/docs/parity-ladder.md`.
- **BONUS BUG found+fixed**: SIGFPE crash on near-full GPU + `-ngl 0` from
  new upstream fit machinery (`common/fit.cpp:408` div-by-zero; coredump
  verified). All degenerate denominators in fit.cpp guarded; repro exits 0.
  Uncommitted pending user review. Upstream-report candidate.
- **State**: daily server running on OLD `build/` binary (restored 17:45);
  candidate `build-ku2/` awaits user's swap call. Uncommitted work:
  oracle tool, parity-ladder doc, fit.cpp guards.

## Decisions

- Bulk merge over 54 cherry-picks; A-list commits individually verified
  in tree post-merge. Proven correct: zero conflicts, all checks green.
- Depth A/B uses q4_0 KV (live config semantics) not tq3_0 (old control's
  llama-bench predates tq3_0 cache-type parsing).
- ccache adopted after user installed it; first-build cost accepted.
- Daily-driver swap deferred to explicit user decision (not silently done).
- fit.cpp guards: minimal denominator checks, degenerate branch falls into
  existing else-log; no math change on non-degenerate path.

## Blockers / open questions

- None blocking. Queued: E2 LUT GEMV (oracle now available for parity
  gate), H1c MoE-specific bench (Qwen3.6-35B), multi-seq depth re-test if
  parallel returns, E5 Vulkan, upstream bug report for fit.cpp SIGFPE.

## Learnings (operational)

- nvcc 12.9 vs system GCC: g++-15/16 headers break CUDA host compile
  (`type_traits(555)` error). `-DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-14`
  mandatory. Reinforces 2026-08-31 standing rule.
- Old-vs-new llama-bench flag drift bites silently: `-fa 0/1`->`on/off`,
  `-dev cuda0`->`CUDA0`, `-ts 22,14` = TWO values (`/` is the separator),
  `--no-cnv`->`-st`. Fingerprint every invocation (P-3 paid off).
- Unpinned dual-backend bench mixes CUDA+Vulkan and costs ~18% tg - always
  `-dev`.
- setsid+disown needed to keep background llama-server alive across tool
  timeouts in this harness.
- `common_init_from_params` runs a warmup decode - capture tools must reset
  state after init (oracle lesson, codified in the tool).
- New upstream `--fit` machinery changes init behavior under memory pressure:
  fit errors log as "encountered an error while trying to fit params" -
  read full logs, not tails.
