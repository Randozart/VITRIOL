# VRAM-Pooling Options Assessment: SwarmLLM.ai vs llama.cpp RPC

**Date:** 2026-08-31
**Context:** User asked whether external VRAM-pooling options (SwarmLLM.ai, llama.cpp distributed/RPC inference, vLLM pipeline parallelism) could give VITRIOL "the best shot at being a great inference machine."

## Verdict at a glance

| Option | Fits VITRIOL? | Reason |
|---|---|---|
| SwarmLLM.ai | **No** | Browser/WebGPU runtime; none of VITRIOL's stack runs there |
| llama.cpp RPC (`tools/rpc`) | **Only with a 2nd LAN machine** | Capacity play, not throughput; useless single-host |
| vLLM pipeline parallelism | **No** | Separate serving stack; vLLM does not run GGUF/VITRIOL CUDA kernels |

## SwarmLLM.ai — rejected

- Fetched https://swarmllm.ai (2026-08-31): browser P2P mesh inference via
  WebGPU + WebRTC; a room of devices each downloads a slice of a fixed model
  (Qwen 3.8-class); "every word of the answer takes a lap through all of them."
- Runs its own WebGPU runtime in-browser. **Zero VITRIOL features apply**:
  no CUDA VITRIOL predictor, no TurboQuant KV (`tq3_0`), no `-ts` dual-GPU
  split, no MTP, no RPC, no GGUF control, no depth certification path.
- Privacy posture ("no servers, words never leave the room") is plausible
  from site copy but performance is capped at browser WebGPU, far below the
  3060/1070 Ti CUDA path.
- **Decision: do not invest time.**

## llama.cpp RPC — viable only with remote hardware

Evidence from this repo (2026-08-31):

- RPC tooling present: `llama.cpp/tools/rpc/rpc-server.cpp` (+ README, CMakeLists).
- Current build does **not** include it: `GGML_RPC:BOOL=OFF`
  (`llama.cpp/build/CMakeCache.txt:809`); `LLAMA_RPC` is a deprecated alias
  for `GGML_RPC` (`llama.cpp/CMakeLists.txt:191`).
- No rpc binary built (`build/bin` contains no rpc target).

Semantics: `rpc-server` on a remote host exposes its VRAM as a remote backend;
the main process pipelines layers to it over TCP. Good for **fitting larger
models**; bad for **decode t/s** (every tensor crossing the wire is a network
hop). On this single host (RTX 3060 + GTX 1070 Ti, `-ts 26,10`) RPC is
strictly worse than the existing local split.

If a second LAN machine with VRAM (or spare system RAM via RPC backend)
becomes available, the plan is:

1. Rebuild with `-DGGML_RPC=ON` and `-DCMAKE_CUDA_ARCHITECTURES="61;86"`
   (per AGENTS.md dual-arch rule).
2. Run `llama-rpc-server` on the remote box; add `--rpc <host>:50052` to the
   launch alongside the certified flags
   (`-ngl 99 -ts 26,10 --main-gpu 0 -ub 64 --cache-type-k tq3_0 --cache-type-v tq3_0`).
3. **Depth-certify**, not just shallow-bench: RPC changes allocation pressure;
   per AGENTS.md rule 5, window ≠ depth. Emit `VITRIOL-FINGERPRINT:` lines
   per rule 4; justify streaming/residency choices per rule 1.

## Bigger lever remains local

The real depth wall is the ~23 KiB/token VRAM creep on dev0 during long
prefills (see `.opencode/plans/lull-certification-report-2026-08-24.md`;
not fixed by `GGML_CUDA_NO_VMM=1`). Fixing that creep yields more usable
depth than any networked VRAM pool, and no added per-token latency.
Best certified local configs to build on:

- UD-IQ3_S + `tq3_0` KV, ts 26,10 ub64: **96,836 tok @ 11.32 t/s**
- Q3_K_M + `tq3_0` KV, ts 26,10 ub64: 54,692 tok @ 9.21 t/s

## Decision

- Skip SwarmLLM (and vLLM) permanently.
- Hold RPC until a second LAN machine exists; then follow the plan above.
- Prioritize local work: VRAM-creep investigation, `tq3_0` KV everywhere,
  quant selection.
