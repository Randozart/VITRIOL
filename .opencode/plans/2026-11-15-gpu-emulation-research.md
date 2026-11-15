# GPU emulation/simulation assessment

Created: 2026-11-15T10:30:11+01:00 (Europe/Amsterdam)

Question: can CPU-side GPU emulation reduce the real-device cost of finding further VITRIOL/Officina optimizations? Preserve inference correctness, ownership/lease/rollback invariants and live service availability. No installation or live benchmark was performed. No repository files edited.

Local observations: repository AGENTS.md identifies RTX3060 + GTX1070Ti, sm86+sm61; this is a documented target, not newly verified live inventory. Existing Spagyric sweep launches actual servers and measures a fixed short prompt. Engine server-common.h exposes cached/processed prompt counts and timing. vitriol-governor/src/observe.rs samples NVML memory/process state through Nvml::init; cached nvml-wrapper supports builder.lib_path, so an injectable library could be selected explicitly in a future adapter. Simply setting DCGM's NVML_INJECTION_MODE is not established to redirect this Rust client. No inference improvement or emulator compatibility is demonstrated by this review.

Candidates and decisions:
- Accel-Sim (community academic project, not NVIDIA-owned): source-level inspection of util/tracer_nvbit/tracer_tool/tracer_tool.cu shows NVBit instrumentation and DYNAMIC_KERNEL_RANGE filtering. Traces record an actual executed path; changed kernels, shapes and execution paths generally need new representative captures. CPU replay can explore microarchitecture configurations, not evaluate arbitrary rewritten CUDA from an old trace. Current config list includes Pascal TITANX and Ampere RTX3070, not a directly validated target-card pair. Tuner README explicitly assumes target hardware, uses microbenchmarks/deviceQuery and searches uncertain parameters. Main README says multi-GPU/NVLink support upcoming. Reuse selectively after calibration; do not adopt as whole-system timing oracle. Main README author-reported H100 correlation is not evidence of accuracy on this mixed pair.
- NVIDIA NVBit: binary instrumentation, real GPU required to capture custom traces; CUDA-EULA research artifact rather than an assumption of a fully open-source emulator. Current requirements include sm3.5 through12.1, Linux, CUDA>=12.0. Capture cost/compatibility must be measured on our actual drivers and kernels. Instrumented elapsed times are not uninstrumented performance.
- NVIDIA DCGM NVML Injection: real open-source injectable fake telemetry. README plus InjectedNvml.cpp LoadFromFile/LoadFromString use YAML inputs, internal fake device/process state and API returns. Useful for reproducible governor memory-pressure/foreign-tenant/error tests; cannot run CUDA kernels or establish a second slot fits. Apache2 headers in inspected source/CMake. A library/observer adapter is needed for our Rust caller; adding full DCGM just for pure-policy tests may cost more than a small sample-replay adapter. Validate the thin observer integration separately if choosing reuse.
- NVIDIA numba-cuda simulator: CUDA-Python semantics, Python CPU interpreter, synchronous streams, no meaningful event timing, one simulatedGPU and mostly unimplemented driverAPI. Not a drop-in runner for ggml CUDA/C++ binaries; reject for this project's existing compiled kernel path.
- GPUOcelot: functional PTX emulator/CPUJIT, README scopes semantics to PTX2.2/Fermi despite recent-toolchain build improvements. Existing VITRIOL kernels use warp shuffles and newer instruction families. Reject as default modern-engine emulator without a representative compatibility demonstration. README screened; no implementation test run.
- PantheonSim (third-party): README and PyTorch documentation screened. Author claims unmodified CUDA/HIP CPU execution, and reports model checks; README explicitly no timing, requires shared cudart, runs kernels synchronously at launch. Worth a bounded correctness experiment on tiny actual ggml kernels; not yet validated for our instructions, CUDA libraries, numerical edge cases or asynchronous races. No installation/test/license adoption decision performed.
- NVIDIA nvMatmulHeuristics: official API docs inspected. Analytical GEMM ranking/runtime estimator with explicit hardware descriptor and discovery calibration, targets CUTLASS/cuBLAS-style kernels, architectures listed Ampere onward. Mechanism is useful prior art for cheap candidate ranking but no direct model of Pascal or custom ggml quantization/attention/PCIe/system scheduling. Module is distributed under software license; no claim all core sources open. Reject as a universal existing-engine predictor.

Recommended experiment order (proposal, not implemented):
1. Capture privacy-conscious request/cache events plus observed GPU memory/allocation/copy/timing facts. Record model/build/layout/hardware identity and event timestamps. Measure tracing overhead.
2. CPU discrete-event replay of session arrivals, queue/slot occupancy, prefix changes, cache tiers, observed memory demand and transfers. Enforce owned-cache retention invariants. Use this to rank policies; tag predictions and uncertainty. Allocation sums are bounds/estimates, not proof of complete runtime fit.
3. Synthetic memory/tenant/telemetry-failure sequences at the governor Sample boundary; optionally use NVML injection for observer ABI tests. Include known pressure spike that should defer a second slot. No process/service actions in replay.
4. Optional functional-emulator smoke test: one real quantized matvec and attention kernel at tiny shapes versus CPU references, numerical tolerances defined, explicit unsupported-op outcomes. Do not use CPU emulator walltime to predict GPU speed; synchronous emulation cannot verify actual stream race freedom.
5. Only if a kernel is a measured bottleneck, capture its bounded NVBit trace separately on each card, calibrate Accel-Sim and compare predicted effects with held-out real runs. Reject a predictor that misranks useful candidates or cannot resolve improvements above its error. Do not reuse one architecture's instruction trace as evidence of different kernels/compiler dispatch on the other.

Search coverage: targeted NVIDIA/NVlabs instrumentation/emulation repositories, Accel-Sim/GPGPU-Sim documentation/config/tracer/tuner source, NVIDIA DCGM fake telemetry implementation, CUDA-Python simulator, legacy PTX CPU emulation, modern third-party CUDA emulator, and analytical GEMM prediction. A guessed NVIDIA/matmulHeuristics raw README URL failed; official nvMatmulHeuristics docs provided the actual mechanism. Some search results were unrelated simulation workloads; not treated as GPU emulators. No exhaustive search or reproduced external benchmarks claimed.

Source ledger, retrieved 2026-11-15 (mutable branches; pin revisions before any installation):
https://raw.githubusercontent.com/accel-sim/accel-sim-framework/dev/README.md
https://raw.githubusercontent.com/accel-sim/accel-sim-framework/dev/util/job_launching/configs/define-standard-cfgs.yml
https://raw.githubusercontent.com/accel-sim/accel-sim-framework/dev/util/tuner/README.md
https://raw.githubusercontent.com/accel-sim/accel-sim-framework/dev/util/tracer_nvbit/README.md
https://raw.githubusercontent.com/accel-sim/accel-sim-framework/dev/util/tracer_nvbit/tracer_tool/tracer_tool.cu
https://raw.githubusercontent.com/accel-sim/accel-sim-framework/dev/gpu-simulator/README.md
https://raw.githubusercontent.com/NVlabs/NVBit/master/README.md
https://raw.githubusercontent.com/NVlabs/NVBit/master/EULA.txt
https://raw.githubusercontent.com/NVIDIA/DCGM/master/nvml-injection/README.md
https://raw.githubusercontent.com/NVIDIA/DCGM/master/nvml-injection/src/InjectedNvml.cpp
https://raw.githubusercontent.com/NVIDIA/DCGM/master/nvml-injection/CMakeLists.txt
https://nvidia.github.io/numba-cuda/user/simulator.html
https://raw.githubusercontent.com/gpuocelot/gpuocelot/master/README.md
https://raw.githubusercontent.com/pantheongpu/pantheonsim/main/README.md
https://raw.githubusercontent.com/pantheongpu/pantheonsim/main/nvidia/docs/pytorch.md
https://docs.nvidia.com/cuda/nvidia-matmul-heuristics/index.html
