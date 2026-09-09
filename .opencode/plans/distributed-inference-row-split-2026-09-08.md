# VITRIOL Distributed Inference — Row-Split RPC (setup + first light)

Date: 2026-09-09
Status: DONE — distributed row-split working end-to-end (box A GPU + box B CPU),
launcher plumbing + fingerprint + profile + box B systemd unit all in place.
Measured: decode 10-11 t/s, prefill 6-20 t/s (box B DDR5-bound). See §5-§6.1.

Goal: transcend one machine — split Qwen3.8-27B (and, by extension, larger
models) across box A (GPU) + box B (CPU/NPU, 64GB unified memory) via the
llama.cpp RPC backend. NOT tensor-parallel; row-split (layer-parallel) only.

## 0. Network + hardware map (credentials withheld)

All traffic over Tailscale (mesh). No public ports, no NAT traversal, no WebRTC.

| role | host | tailscale IP | OS | CPU | memory | accelerator |
|---|---|---|---|---|---|---|
| A (this box) | randy-pc-1 | 100.111.244.0 | Linux | i7-3770 | 16 GB DDR3 | RTX 3060 12 GB + GTX 1070 Ti 8 GB (CUDA) |
| B | overlord-x8664 | 100.92.76.67 | CachyOS (Arch) | Core Ultra X7 358H (Panther Lake, 16C) | 64 GB DDR5 | Intel Arc B390 + NPU (50 TOPS) |
| C (unused) | laptop-0lf9bbpc | 100.124.222.109 | Windows | ThinkStation | ? | ? (ports closed / offline) |

RTT: box A <-> box B = 13 ms (tailscale direct). Box C = 13 ms via DERP relay,
all ports closed (not usable right now).

SSH to box B: `prestopoverlord@100.92.76.67` (password auth via
`SSH_ASKPASS` — see helper below; do NOT commit the password).

## 1. Why row-split RPC (not tensor-parallel, not WebRTC)

- Tensor-parallel across machines: one allreduce per layer boundary, in-kernel
  PCIe spin design (`ggml-cuda/allreduce.cu`) is same-machine-only. Over a
  socket it's dozens of 100s-µs-to-ms round-trips per token = exceeds the
  ~77 ms/token budget at 13 t/s. Latency wash.
- WebRTC: solves NAT traversal + lossy-stream framing (audio/video, FEC,
  jitter buffers). Unneeded on a Tailscale mesh; the workload is a few-KB
  tensor sent as ordered, zero-loss request/response. Tailscale TCP is the
  correct transport.
- RPC backend (`ggml-rpc`) is raw length-prefixed TCP with FNV-hash weight
  dedup — already present in this fork, already wired (`--rpc` CLI flag,
  `tools/rpc/rpc-server.cpp`).

## 2. The RPC backend in this fork

- Source: `llama.cpp/ggml/src/ggml-rpc/` (upstream ggml-rpc).
- Build gate: `GGML_RPC=ON` (default OFF).
- Server binary: `ggml-rpc-server` (note: NOT `rpc-server` — that target does
  not exist; it is `tools/rpc/CMakeLists.txt:1` `set(TARGET ggml-rpc-server)`).
- Client flag: `--rpc host:port` (`common/arg.cpp:1171`), gated on
  `llama_supports_rpc()`.
- Device placement: RPC servers are inserted at the FRONT of the device list
  (`src/llama.cpp:276`), so `-ts` (tensor_split) needs one entry per device:
  `[RPC(boxB), GPU0(3060), GPU1(1070Ti)]` = 3-way split.
- Weight transport: `RPC_CMD_SET_TENSOR` once per unique tensor, FNV-hash
  dedup (`ggml-rpc.cpp:700-722`); activations via `COPY_TENSOR`/`GRAPH_COMPUTE`.

## 3. Op-count assertion fix (VITRIOL-specific, already applied on box B)

VITRIOL adds GGML ops (op count 102 vs upstream 101). The RPC protocol header
asserts the count to keep both sides in sync:
`llama.cpp/ggml/include/ggml-rpc.h:14`:
```
static_assert(GGML_OP_COUNT == 101, "...");
```
When a new op is added, bump this to the current count:
```
sed -i 's/GGML_OP_COUNT == 101/GGML_OP_COUNT == 102/' ggml/include/ggml-rpc.h
```
Keep `RPC_PROTO_PATCH_VERSION` bumped when the WIRE format actually changes
(op additions that change the graph serialization), not just the count. As of
2026-09-09 the count assert is the only stale guard; patch version stays 0.

## 4. Working setup (box B)

```
# build (one-time)
cd ~/Projects/VITRIOL/llama.cpp
cmake -B build-sycl -DGGML_RPC=ON          # preserves existing SYCL config
cmake --build build-sycl --target ggml-rpc-server -j$(nproc)

# run (needs no sudo; ufw inactive)
./build-sycl/bin/ggml-rpc-server -H 0.0.0.0 -p 50052
```

Observed (2026-09-09):
```
Starting RPC server v6.0.0
  endpoint       : 0.0.0.0:50052
Devices:
  CPU: Intel(R) Core(TM) Ultra X7 358H (63781 MiB, 63781 MiB free)
  transport      : TCP
```
- `ggml_sycl_init: no SYCL device available` is a benign warning when the
  oneAPI env is not sourced; the CPU device still registers. The Arc B390 GPU
  is NOT exposed by the rpc-server's default device enumeration (CPU only) —
  use `-d` to force devices if GPU offload is desired.
- The `0.0.0.0` security warning is expected; traffic is confined to the
  Tailscale mesh (no public exposure).

## 5. Box A setup — DONE (2026-09-09)

1. Built with RPC: `cmake -B build -DGGML_RPC=ON` (box A build dir).
   Also applied the op-count fix in `ggml/include/ggml-rpc.h` (see §3) and
   bumped `RPC_PROTO_PATCH_VERSION` 0->1 so both sides match v6.0.1.
2. Launch: `llama-server --rpc 100.92.76.67:50052 -m <model> -ngl 65 -ts <3-way> ...`
3. **Smoke tested end-to-end**: coherent output on box A through the split;
   box B log shows `Accepted client connection` + ESTAB from box A
   (100.111.244.0 -> 100.92.76.67:50052).
4. Measured profile (2026-09-09, Qwen3.8-27B, box B = overlord CPU over
   Tailscale 13 ms RTT):

| config | decode t/s | prefill t/s |
|---|---|---|
| local blessed (24,12 + MTP) | 18.94 | 34 |
| RPC 5% box B (0.05,0.475,0.475) | 11.35 | 7.66 |
| RPC 25% box B (0.25,0.375,0.375) | 10.03 | 6.03 |
| RPC 40% box B (0.40,0.30,0.30) | 4.87 | 4.87 |

   Sweet spot for decode: small box B slice (~5%). Prefill is box-B
   DDR5-bandwidth-bound (~3 GB slice streamed per token) -> ~20 t/s at 25%
   slice (405 s for 8K tokens). **20 t/s prefill deemed acceptable** by user.
   Note: RPC device sits at FRONT of the device list, so box B's layers process
   every prefill/decode token (position does not amortize).

## 6. Launcher + fingerprint plumbing — DONE (2026-09-09)

- `scripts/vitriol`: added `[gpu] rpc_servers = <host:port>` config key
  (parse_config, config_set, write_config, config_show), emitted `--rpc` into
  all 4 server launch sites (memory + external, detach + foreground).
- Fingerprint: `rpc=<host:port>` appended conditionally in the two serve paths
  AND `config_fingerprint` (for `config bless` parity). Topology-bearing key,
  REQUIRED by flag-provenance rule (AGENTS.md).
- `-ts` becomes 3-way: `[boxB_frac, 3060_frac, 1070Ti_frac]` summing to 1.0.
- Bundled profile `profiles/qwen38-distributed/` saved: 5% box B slice
  (0.05,0.475,0.475) + `rpc_servers = 100.92.76.67:50052`.

## 6.1 Box B rpc-server lifecycle — DONE (2026-09-09)

- Replaced the ad-hoc `nohup` run with a user systemd unit
  (`scripts/systemd/vitriol-rpc-server.service`, installed at
  `~/.config/systemd/user/vitriol-rpc-server.service` on box B).
- `systemctl --user enable --now vitriol-rpc-server.service`; active, listening
  on 0.0.0.0:50052, survives SSH disconnect + login (Linger=yes on box B).
- Bind to 0.0.0.0 so box A reaches it via the Tailscale mesh address.

## 7. Honest expectations — MEASURED (2026-09-09)

- box B's compute is CPU (SYCL, `-ngl 0`). It already runs a 20.9B MoE at
  25 t/s CPU-only; for 27B dense (all params active) expect slower per-layer.
- Decode = box_A_layers + box_B_layers + 2 x 13 ms network. **Measured
  10-11 t/s at 5-25% box B slice** vs 18.94 local. **Prefill 6-20 t/s**,
  DDR5-bandwidth-bound on box B. Confirmed: the win is CAPACITY (box B's
  64 GB DDR5 for KV/context), not speed.
- **Prefill-on-A / KV-ship-to-B** (send prefilled data over the wire): the RPC
  `SET_TENSOR` primitive already uploads arbitrary tensors to box B buffers
  (that's how weights load), and KV cache for box B's layers is allocated on
  box B's RPC buft (`llama-kv-cache.cpp:218`). A "prefill all layers on box A,
  then ship box B's KV in one bulk transfer" mode is a real option if the 20 t/s
  prefill ever becomes the blocker. NOT implemented (20 t/s accepted). Requires:
  (1) a prefill phase that runs all layers on box A (needs full weights in box A
  VRAM, ~12 GB, tight but feasible at moderate ctx), (2) a KV-upload path reusing
  `RPC_CMD_SET_TENSOR`. `COPY_TENSOR` is box-B-local only (same-dispatcher), so
  cross-host ship must go through `SET_TENSOR`.
- NPU: not usable for 27B (SYCL backend GPU-only; OpenVINO NPU path limited to
  ~1-3B models, static-graph, stateless). Revisit if a small draft model on NPU
  is ever wanted for speculative decoding.
- **CUDA Rust (NVIDIA, Sep-2026, future direction)**: two tracks — cuda-oxide
  (SIMT kernels in Rust, nightly + custom LLVM, early alpha) and cutile-rs
  (Tile-based, stable Rust 1.89+, CUDA 13.3, used by HuggingFace Grout +
  mistral.rs). NOT applicable to VITRIOL now: both require CC 8.0+ (Ampere+);
  the 1070 Ti is CC 6.1, and box B is Intel Arc/SYCL (NVIDIA-only toolchain).
  Integration into `ggml-cuda` (monolithic C++ template-instantiated kernels)
  would need a separate backend, not a swap. File under future direction; the
  mature entry point is cutile-rs if a standalone Rust inference engine ever
  emerges (VITRIOL's Rust already exists in `libvitriol/`, host-side only).

## 8. Credentials handling (do not commit)

SSH to box B uses a password. For scripted use from box A, use `SSH_ASKPASS`:

```python
pw = tempfile.NamedTemporaryFile(mode='w', suffix='.sh'); pw.write('#!/bin/sh\necho <PW>\n')
env['SSH_ASKPASS'] = pw.name; env['SSH_ASKPASS_REQUIRE'] = 'force'; env['DISPLAY'] = ':0'
```

Keep the password out of the repo. Add box A's public key to box B's
`~/.ssh/authorized_keys` to make SSH key-based instead (recommended follow-up).

## 9. Traceability

- `.opencode/plans/distributed-inference-row-split-2026-09-08.md` — design plan
  (superseded by this doc for the SETUP stage).
- RPC source: `llama.cpp/ggml/src/ggml-rpc/`; server: `tools/rpc/rpc-server.cpp`.
- Op-count guard: `llama.cpp/ggml/include/ggml-rpc.h:14`.
- Box B build: `~/Projects/VITRIOL/llama.cpp/build-sycl/` (GGML_RPC=ON).