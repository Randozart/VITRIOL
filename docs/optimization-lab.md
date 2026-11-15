# Optimization laboratory

Created: 2026-11-15, Europe/Amsterdam. Initial contract; implementation is planned.

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
