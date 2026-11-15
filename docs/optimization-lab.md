# Optimization laboratory

Created: 2026-11-15, Europe/Amsterdam. Offline replay and bounded correctness adapters are implemented.

The lab is an offline experiment layer for VITRIOL. It compares session/cache
policies with deterministic CPU replay, feeds synthetic GPU-pressure samples to
the real governor policy, and checks numerical results through bounded external
correctness backends. It never controls the active engine.

The [research assessment](../.opencode/plans/2026-11-15-gpu-emulation-research.md)
records the sources, examined code, alternatives and limitations. The
[implementation plan](../.opencode/plans/2026-11-15-102907-optimization-lab.md)
records invariants and verification obligations.

Three evidence classes stay distinct: synthetic/replayed policy results,
functional-emulator correctness, and measured hardware results. CPU/emulator
elapsed time is not GPU performance. A user-provided cost model or GPU allocation
estimate does not establish physical capacity or improvement.

The first replay uses supplied prefix-equality IDs and model/layout identities,
whole snapshot entries, bounded RAM/disk tiers, explicit protection flags and
per-request prefill/restore/decode costs. Completions publish state before later
requests may reuse it. Slot selection and eviction policy are experiment inputs.
Prefixes represent observed state; the simulator does not generate tokens.
Trace IDs and timing/shape metadata can still be sensitive and stay private.

Predicted policy rankings require comparison with held-out real measurements.
Full GPU instruction simulation is a selective later experiment: Accel-Sim
requires representative captures and per-card calibration; its architecture
defaults do not validate the documented RTX 3060 + GTX 1070 Ti pair. NVIDIA
NVML Injection can exercise the observer interface, but pure governor sample
replay avoids that dependency for most policy tests. Optional Numba simulation
checks CUDA-Python algorithms, not VITRIOL's compiled CUDA/C++ kernels.

Build and install the CLI, then run the synthetic examples from the repository root:

```sh
cargo build --release --locked --offline --manifest-path vitriol-lab/Cargo.toml
install -m 755 vitriol-lab/target/release/vitriol-lab ~/.local/bin/vitriol-lab
vitriol lab compare vitriol-lab/examples/sessions.json
vitriol lab governor vitriol-lab/examples/pressure.json
vitriol lab kernel-check vitriol-lab/examples/kernel-reference.json
vitriol lab validate vitriol-lab/examples/calibration.json
```

The session fixture compares baseline, RAM, disk-tier and two-slot candidates.
The two-slot candidate exceeds the smaller GPU's supplied memory estimate and
cannot become `fastest_feasible_estimate`, even if its predicted timing wins.
Missing GPU inputs produce `unknown` capacity and no feasible-estimate winner.
These numbers are deliberately tiny synthetic inputs, not measurements of the
operator's hardware. Governor output records `intended_actions` and always
`executed_actions: 0`; it runs the same policy library as the daemon.

The independent Rust CLI supports the same commands directly as `vitriol-lab`.
`VITRIOL_LAB_BIN` selects an explicit binary for the launcher. The external
correctness runner currently requires a Unix system with `waitid/WNOWAIT`;
the tested platform is Linux. There are no network calls in the lab itself.

## Session trace contract

Use [sessions.json](../vitriol-lab/examples/sessions.json) as the complete version-1
schema example. `replay` accepts the same envelope with exactly one candidate;
`compare` accepts 1–64 uniquely named candidates. Input files are bounded to
16 MiB, traces to 10,000 requests, slots to 16 and resident entries to 4,096.
Unsupported versions, unknown fields, duplicate IDs, out-of-order arrivals and
arithmetic overflow are errors. All time quantities are integer microseconds.

| Input | Meaning |
|---|---|
| `provenance.origin` | `synthetic` or `recorded`; this is the producer's declaration |
| `provenance.costs` | `assumed` or `measured`; attach matching build/hardware fingerprints |
| `prefix_ids` | Equality-preserving IDs for the request's token prefix |
| `state_prefix_ids` | Observed prefix that will be available after successful completion |
| `snapshot_bytes` | Declared captured-state size; zero publishes no cache entry |
| `protected` | Retain this entry for the whole trace; active cache readers also prevent eviction |
| `prefill_us_per_token` | Declared linear prefill coefficient for this request |
| `decode_us` | Declared positive decode duration, independent of reuse |
| `ram_restore_us`, `disk_restore_us` | Declared restoration cost from each tier |
| `reprocess_tokens` | Required uncached tail even on an otherwise complete hit |
| `gpu_external_bytes` | Optional override covering every GPU at this arrival |

Requests arrive in trace order, including equal-time arrivals. Every completion
at a timestamp is processed in slot-index order before new arrivals at that
timestamp. Queued requests remain FIFO. `dispatch: affinity` chooses the cheapest
estimated start among currently free slots for the first queued request; it
does not reorder the queue or wait for a busy warm slot. A slot's state becomes
visible only on completion. Cache restoration is chosen only when it costs less
than processing the corresponding tokens; model/layout and rewind checks apply.

RAM entries demote to disk under pressure, then eligible disk victims disappear
from the simulation. `eviction: lru` uses last access; `value` prefers retaining
the largest declared full-recomputation cost per byte, with recency and entry ID
as deterministic tie-breakers. This is a basic heuristic, not a learned reuse
probability or an implementation of GreedyDual-Size. [Cost-aware caching prior
art](https://www.usenix.org/legacy/publications/library/proceedings/usits97/full_papers/cao/cao_html/node8.html)
motivates comparing cost with size; no external implementation is copied.

Budgets and entry-count limits retain protected/read entries and skip admission
when they cannot make room. These are hypothetical residency changes: the lab
has no authority to unlink engine snapshots. Protection inputs must reflect the
retention contract being evaluated. Branch retirement and writer fencing still
belong to the engine-owned Rust lifetime store; this first replay does not
simulate that full ledger, failure/cancellation/retry behavior, CUDA allocation
peaks, transient staging buffers or PCIe/concurrent-kernel overlap.

Reports include queue time, cached/processed tokens, predicted prefill cost,
makespan, p95 latency, tier counters and estimated capacity violations. Miss
categories are model-derived explanations, not evidence of the native engine's
exact failure cause. GPU requirements are checked separately for each device:
`fixed_bytes + slots × slot_bytes + external_bytes <= capacity_bytes`. They are
input estimates, not a proof that another slot physically fits. This version
accepts prepared traces; it does not yet capture real session prefixes/timings
automatically. Avoid publishing raw traces or pretending synthetic coefficients
are measurements. Prefix IDs can reveal structure even without plaintext.

CLI reports bind exact input bytes and argv. Replay also hashes canonical trace
and candidate JSON so whitespace differences do not change semantic identity.
The algorithm scans bounded cache/history candidates rather than running a
cycle-level GPU model. Its purpose is cheap policy exploration; no speedup of
the actual inference engine has been established by the synthetic examples.

## Governor scenarios

[pressure.json](../vitriol-lab/examples/pressure.json) feeds timestamped samples
to `vitriol_governor::policy::Policy::decide`. `config` uses the daemon's existing
defaults and optional overrides. An optional initial `policy` can represent a
checkpoint; its times must not lie after the first sample. Empty telemetry does
not invent free memory. Replay observes actual policy behavior, including its
dwell/pressure/recovery rules; it does not assume that an intended resize/yield
succeeds or adjust subsequent samples to simulate its effects. No engine HTTP,
NVML initialization, profile changes or systemd execution occur.

## Numerical correctness backends

[kernel-reference.json](../vitriol-lab/examples/kernel-reference.json) checks
matvec, dot and stable softmax against an independent f64 Rust oracle. `program:
"self"` is reserved for the CPU reference protocol with `args: ["reference"]`.
Passing it verifies the runner/oracle path, not GPU compatibility.

External backends use an absolute `program`, explicit argument array, environment
overrides and an `artifacts` list. Include the adapter, compiled kernel, emulator
libraries and environment lockfiles needed to identify the experiment. The
runner hashes regular files up to 512 MiB before and after the suite and rejects
changed artifacts. It does not discover every transitive dependency or certify
the supplied backend kind. Reports always keep `hardware_verified: false` and
`gpu_timing_available: false`, including `declared_hardware` backends.

Each case starts a fresh process in a private temporary directory. Its stdin is
only the operation's JSON, never the expected answer. The worker writes one JSON
object to stdout:

```json
{"version":1,"output":[17,39],"backend":"my-functional-emulator revision ..."}
```

Operation schemas are shown in the fixture. Output length and every value must
match the oracle within `abs_tolerance + rel_tolerance × abs(reference)`;
nonfinite values, malformed responses and nonzero exits fail. Child exit 77
means unavailable. Timeouts are 1–60,000 ms per case; input and each output stream
are bounded to 1 MiB. The Linux runner retains child ownership until process-group
cleanup, kills the group on timeout and after completion, and limits output files.
It is for trusted programs; these resource limits are not a security sandbox.
`runner_wall_us` measures host execution only and must never rank GPU kernels.

The optional [Numba adapter](../vitriol-lab/adapters/numba_sim.py) implements small
CUDA-Python algorithm checks. It requires `NUMBA_ENABLE_CUDASIM=1`, otherwise exits
77 before selecting hardware. Use a separately prepared compatible Python/Numba
environment, an absolute interpreter and absolute adapter path, and hash the
adapter plus its environment lockfile in `artifacts`. Change the reference suite's
backend to `functional_emulation`, program to that interpreter, args to the adapter
path, and env to `{"NUMBA_ENABLE_CUDASIM":"1"}`. These are generic numerical
algorithms, not exported ggml kernels. Compiled CUDA/Accel-Sim/Pantheon backends
need their own protocol worker and compatibility/correctness validation.

On this development host the Numba package is absent; a package-metadata network
probe failed with DNS resolution unavailable. No Numba execution or GPU inference
is claimed. Numba-CUDA's current [package metadata](https://pypi.org/pypi/numba-cuda/json)
also announces maintenance mode and points new work toward Numba-CUDA-MLIR; the
adapter is optional rather than a new runtime dependency.

## Validate predictions against held-out measurements

[calibration.json](../vitriol-lab/examples/calibration.json) is a synthetic test
of the validator, not a hardware result. Supply candidate and baseline predictions
and independent measurements from matching model/build/hardware, with
`measurement_origin: recorded`, `held_out: true`, and the evaluated trace hash.
These declarations are not physical attestation. The validator checks every
candidate and baseline against `max_relative_error`, then checks candidate
ordering relative to the named baseline. An effect is unresolved if it does not
exceed both the minimum-effect threshold and the conservative error envelope
`max_relative_error × (baseline measured time + candidate measured time)`.
If all comparisons are unresolved the result is `inconclusive`, never a winner.

Exit codes are 0 for a successful experiment/check, 1 for invalid/failed checks,
and 2 for unavailable or inconclusive checks. A successful replay predicts; it
does not establish a real speedup. Use the same trace across candidates and
separate calibration from held-out evaluation before changing production policy.

## Small decision models: candidate follow-up

Jev-style typed models could replace some semantic yes/no, scoring and bounded
selection calls. The existing Officina auto-route classifier already uses cheap
rules, so replacing that does not eliminate an LM call. Keep admission arithmetic,
retry protocol and cache-retirement authority deterministic. Promising semantic
experiments include expensive-review gating, retrieval-source selection and
bounded tool/task classification, with uncertain decisions falling back.

[Laya](https://huggingface.co/convaiinnovations/laya) is an open-weight candidate;
its [backend implementation](https://github.com/NandhaKishorM/laya/blob/main/laya/backends/__init__.py)
offers a CPU ONNX path. [AnyJev](https://github.com/nokia-applied-research/AnyJev)
supports typed readout from open LMs; cost depends on the actual model and inference
method. [Ollaya](https://github.com/ollaya-dev/ollaya) provides a local serving
surface. These are screened candidates, not adopted or benchmarked dependencies.

Evaluate compact structured state already available in code rather than adding
an LM summarization call or repeatedly reading the full session. Shadow-score
real decisions for accuracy, calibration, abstention/fallback rate, CPU/RAM use,
latency and net avoided LM work. Include quantization and option-order robustness.
Keep the main local model/KV resident while measuring a CPU-first candidate.
Generic confidence claims do not establish calibration on our traffic; the lab
currently has no decision-model evaluator or live routing integration.
