# VITRIOL Distributed Inference — Row-Split RPC

Date: 2026-09-09
Status: WORKING (box A GPU + box B CPU, measured). Canonical reference for
distributed operation; supersedes the plan note at
`.opencode/plans/distributed-inference-row-split-2026-09-08.md`.

## What it is

Row-split (layer-parallel) inference: one model runs across **two machines**
over the llama.cpp RPC backend. Each machine computes the layers it owns;
whole layers (not tensor rows) are offloaded. The transport is raw TCP over a
Tailscale mesh — no WebRTC, no NAT traversal, no public exposure.

The win is **capacity**, not speed: box B's large unified memory holds KV
cache / context that box A's VRAM cannot.

## Topology (anonymized)

| role | host | accelerator | memory | notes |
|---|---|---|---|---|
| box A (local) | randy-pc-1 | RTX 3060 12 GB + GTX 1070 Ti 8 GB (CUDA) | 16 GB DDR3 | runs llama-server (client) |
| box B (remote) | overlord-x8664 | Intel Core Ultra X7 358H (16C) CPU | 64 GB DDR5 | runs ggml-rpc-server |
| box C (unused) | thinkstation | unknown | unknown | ports closed / offline |

Box A <-> box B: 13 ms RTT over the Tailscale mesh. Mesh addresses are
100.x.y.z (Tailscale CGNAT range) — substitute your own; none are published.

## Why row-split, not tensor-parallel or WebRTC

- **Tensor-parallel across machines**: one allreduce per layer boundary, and
  the in-kernel PCIe spin design (`ggml-cuda/allreduce.cu`) is same-machine
  only. Over a socket that is dozens of 100s-µs-to-ms round-trips per token —
  exceeds the ~77 ms/token budget at 13 t/s. Latency wash.
- **WebRTC**: solves NAT traversal (ICE/STUN/TURN) + lossy-stream framing
  (audio/video, FEC, jitter buffers). Unneeded on a Tailscale mesh; the
  workload is a few-KB tensor sent as ordered, zero-loss request/response.
  Tailscale TCP is the correct transport.
- **RPC backend** (`ggml-rpc`) is raw length-prefixed TCP with FNV-hash weight
  dedup — already present in the fork, already wired (`--rpc` CLI flag,
  `tools/rpc/rpc-server.cpp`).

## Measured profile (2026-09-09, Qwen3.8-27B, box B CPU over 13 ms mesh)

| config | decode t/s | prefill t/s |
|---|---|---|
| local blessed (24,12 + MTP) | 18.94 | 34 |
| RPC 5% box B (0.05,0.475,0.475) | 11.35 | 7.66 |
| RPC 25% box B (0.25,0.375,0.375) | 10.03 | 6.03 |
| RPC 40% box B (0.40,0.30,0.30) | 4.87 | 4.87 |

- Sweet spot for decode: small box B slice (~5%).
- Prefill is **box B DDR5-bandwidth-bound**: box B streams its ~3 GB layer
  slice from RAM for every prefill token (~60 ms/token at a 25% slice).
  Batching does not fix this — the weights must move regardless. 20 t/s
  prefill is the practical ceiling; judged acceptable.
- The RPC device sits at the FRONT of the device list, so box B's layers
  process every prefill/decode token (position does not amortize the cost).
- Verdict: trade ~45% decode speed for box B's 64 GB RAM (context/KV
  capacity). 20 t/s prefill is the accepted operating point.

## Setup

### Build (both sides)

Requires `-DGGML_RPC=ON`. Both ends must carry the same RPC protocol version.

```sh
# box A (CUDA)
cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86" -DGGML_RPC=ON
cmake --build build --target llama-server ggml-rpc-server

# box B (SYCL/CPU; preserves existing config)
cmake -B build-sycl -DGGML_RPC=ON
cmake --build build-sycl --target ggml-rpc-server
```

### Op-count assertion (VITRIOL-specific)

VITRIOL adds GGML ops (`GGML_OP_COUNT` 102 vs upstream 101). The RPC protocol
header guards this in `ggml/include/ggml-rpc.h`:

```sh
sed -i 's/GGML_OP_COUNT == 101/GGML_OP_COUNT == 102/' ggml/include/ggml-rpc.h
sed -i 's/RPC_PROTO_PATCH_VERSION    0/RPC_PROTO_PATCH_VERSION    1/' ggml/include/ggml-rpc.h
```

Keep the patch version bumped when the wire format changes (op additions that
alter graph serialization). Both ends must match; mismatch surfaces as
`Expected HELLO command` / version rejection in the server log.

### Box B: run the RPC server

Ad-hoc, or as a user systemd unit (`scripts/systemd/vitriol-rpc-server.service`):

```sh
./build-sycl/bin/ggml-rpc-server -H 0.0.0.0 -p 50052
```

Observed startup:
```
Devices:
  CPU: Intel(R) Core(TM) Ultra X7 358H (63781 MiB, 63781 MiB free)
  transport      : TCP
```

- `ggml_sycl_init: no SYCL device available` is a benign warning when the
  oneAPI env is not sourced; the CPU device still registers.
- The `0.0.0.0` security warning is expected; traffic is confined to the
  Tailscale mesh (no public exposure).
- The systemd unit (`~/.config/systemd/user/vitriol-rpc-server.service`) keeps
  it alive across SSH disconnect/reboot (user `Linger=yes`).

### Box A: launch the server

```sh
llama-server --rpc <box-b-mesh-ip>:50052 \
  -m <model> -ngl 99 -ts 0.05,0.475,0.475 --main-gpu 1 ...
```

- `--rpc` is gated on `llama_supports_rpc()`; absent from `--help` unless the
  build has RPC.
- The device list becomes `[RPC(boxB), GPU0, GPU1]`; `-ts` is a **3-way split**
  with one fraction per device, summing to 1.0.
- Weights ship once (FNV-hash dedup); per-token cost is one hidden-state
  round-trip (~10 KB f16 for n_embd 5120).

## Launcher integration

- `scripts/vitriol`: `[gpu] rpc_servers = <host:port>` config key emits `--rpc`
  in all 4 launch sites (memory + external, detach + foreground).
- Fingerprint: `rpc=<host:port>` appended conditionally in the serve paths AND
  `config_fingerprint` (for `config bless` parity). Topology-bearing key,
  REQUIRED by the flag-provenance rule (AGENTS.md) — `rpc=` must appear in
  every launch fingerprint and benchmark argv.
- Bundled profile `profiles/qwen38-distributed/`: 5% box B slice
  (0.05,0.475,0.475) + `rpc_servers`. Load with `vitriol config load
  qwen38-distributed`.

## Honest limits

- Decode ~10-11 t/s (vs 18.94 local); prefill 6-20 t/s (box B DDR5-bound).
- box B's compute is CPU (SYCL, `-ngl 0`); no CUDA/GPU offload there.
- The NPU on box B is not usable for 27B models: the SYCL backend targets GPU
  only, and the OpenVINO NPU path is limited to ~1-3B models (static-graph,
  stateless).

## Parked options

### Prefill-on-A / ship-KV (fix the prefill wall)

The RPC `SET_TENSOR` primitive already uploads arbitrary tensors to box B
buffers (that is how weights load), and KV cache for box B's layers is
allocated on box B's RPC buft (`llama-kv-cache.cpp:218`). A "prefill all
layers on box A, then ship box B's KV in one bulk transfer" mode is real if the
20 t/s prefill ever becomes the blocker. NOT implemented (20 t/s accepted).
Requires: (1) a prefill phase running all layers on box A (~12 GB weights in
box A VRAM, tight but feasible at moderate ctx), (2) a KV-upload path reusing
`RPC_CMD_SET_TENSOR`. `COPY_TENSOR` is box-B-local only (same-dispatcher), so
cross-host ship must go through `SET_TENSOR`.

### CUDA Rust (NVIDIA, Sep-2026)

Two tracks — cuda-oxide (SIMT kernels in Rust, nightly + custom LLVM, early
alpha) and cutile-rs (Tile-based, stable Rust 1.89+, CUDA 13.3, used by
HuggingFace Grout + mistral.rs). NOT applicable to VITRIOL now: both require
CC 8.0+ (Ampere+); the 1070 Ti is CC 6.1, and box B is Intel Arc/SYCL
(NVIDIA-only toolchain). Integration into `ggml-cuda` (monolithic C++
template-instantiated kernels) would need a separate backend, not a swap. The
mature entry point is cutile-rs if a standalone Rust inference engine ever
emerges (VITRIOL's Rust exists in `libvitriol/`, host-side only).

## Traceability

- Design + setup: `.opencode/plans/distributed-inference-row-split-2026-09-08.md`.
- RPC source: `llama.cpp/ggml/src/ggml-rpc/`; server: `tools/rpc/rpc-server.cpp`.
- Op-count guard + protocol bump: `llama.cpp/ggml/include/ggml-rpc.h`
  (committed `a6bf4a2fe`).
- Launcher plumbing + profile + systemd unit: outer `5f1e932`.
- Measured 2026-09-09, box B = overlord-x8664 (CPU) over Tailscale.