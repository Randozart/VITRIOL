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
  `.opencode/plans/e1-upstream-sync-2026-09-01.md`. H1c BLOCKED (no MoE
  model loadable by both builds; user has Models/ on an unmounted external
  drive - mount to unblock via Mellum2 or a future 35B).
- **E3 EXECUTED** (oracle + parity ladder): new `tools/vitriol-oracle`
  capture tool + `diff.py`; all gates green. Ladder adopted:
  `llama.cpp/docs/parity-ladder.md`.
- **fit.cpp SIGFPE found+fixed** (upstream #27169-adjacent); complementary
  2-site branch `fix-fit-degenerate-divisors` prepared, USER committed
  (their message) and pushed to fork (`08033df0`) for their own PR.
- **E2c STARTED** (27B decode audit): sm_61 mmvq headroom CONFIRMED -
  1070 Ti at 66% of bandwidth peak vs 3060 at 81% on identical q6_K 9B
  workload; +23% available at parity. Dispatch audit mid-flight.
- **tq3_0 PORT REGRESSION CLOSED** (evening): the certified TurboQuant KV
  was unreachable on the vitriol-ku line - COUNT=43 vs TQ types at
  44/46/200 (OOB), type_traits + CPU traits + CPU quants + whitelists all
  dropped in the port. Five-layer restoration from the frozen `vitriol`
  branch; server boots clean. E-KV-0 depth measurement aborted mid-run
  (user); deferred. EXPERIMENT_LOG (7) has the full fix map.
- **Side track SHELVED**: Briev-native inference + custom format program -
  full discussion, ladder B0-B5, weight-format math (TQ2_0 27B = 7.04 GB =
  single-1070-Ti driver at ~2x decode), open questions:
  `.opencode/plans/briev-inference-format-side-track-2026-09-01.md`.
- **State at day end**: daily server RUNNING on old `build/` binary
  (q4_0 KV, restored 22:30); candidate `build-ku2/` = post-fix tq3_0-
  capable; tq3_0 fix + oracle UNCOMMITTED on inner main; daily-driver
  swap still user's call.

## Decisions

- Bulk merge over 54 cherry-picks; A-list commits individually verified
  in tree post-merge. Proven correct: zero conflicts, all checks green.
- Depth A/B uses q4_0 KV (live config semantics) not tq3_0 (old control's
  llama-bench predates tq3_0 cache-type parsing).
- ccache adopted after user installed it; first-build cost accepted.
- Daily-driver swap deferred to explicit user decision (not silently done).
- fit.cpp guards: minimal denominator checks, degenerate branch falls into
  existing else-log; no math change on non-degenerate path.
- Upstream-bound commit etiquette held: branch prepared, USER wrote the
  commit message and pushed; no AI push, no AI PR.
- tq3_0 restoration sourced from the frozen `vitriol` branch verbatim
  (not re-derived) - registration values are contract, not invention.
- User aborted the long E-KV-0 server-path measurement; recorded as
  deferred, not failed. Baselines intact for the next window.

## Blockers / open questions

- None blocking. Queued: commit tq3_0 fix + oracle (user approval),
  E-KV-0 depth window (~10 min), E2c resume (dispatch audit points:
  GENERIC mmvq table on sm_61, slow_pascal small_k disables, GB10-only
  halve_iters), H1c unblock (mount external drive), daily-driver swap
  decision, E5 Vulkan, E7 dp4a, E8 KV-PQ offline probe.

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
  timeouts in this harness; tool kills the process group otherwise.
- `common_init_from_params` runs a warmup decode - capture tools must reset
  state after init (oracle lesson, codified in the tool).
- Port-regression pattern (IMPORTANT): a rebased feature whose enum values
  exceed a rebased GGML_TYPE_COUNT fails ONLY at runtime in Release
  (asserts compiled out) and ONLY when a code path touches type_traits -
  and an upstream-style arg whitelist can mask it for months by rejecting
  the type up front. Grep for enum-value-vs-COUNT drift when porting
  quant types. The 2026-08-24 tq3_0 certifications could NOT have run on
  today's main - cert provenance matters when rebasing.
- Token math for prompt-filling: chars/4 approximations mislead; the server
  reports prompt_n - trust it, and size fill prompts from it.
