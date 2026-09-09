# VITRIOL Distributed Inference — Row-Split RPC (setup + first light)

Date: 2026-09-09
Status: SETUP COMPLETE — box B rpc-server running; box A build + smoke pending

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

## 5. Pending (box A)

1. Build with RPC: `cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86" -DGGML_RPC=ON`
2. Launch: `llama-server --rpc 100.92.76.67:50052 -m <model> -ngl 65 -ts <3-way> ...`
3. Verify in load log that layers offload to the RPC device.
4. Measure decode t/s + find context-capacity ceiling (window != depth rule:
   report FILLED tokens, not allocated ctx).

## 6. Launcher + fingerprint plumbing (planned)

- `scripts/vitriol`: add `[gpu] rpc_servers = <host:port>` config key, emit
  `--rpc` into server argv.
- Fingerprint (`scripts/vitriol:111`): add `rpc=<host:port>` — topology-bearing
  key, REQUIRED by the flag-provenance rule (AGENTS.md).
- `-ts` becomes 3-way: `[boxB_frac, 3060_frac, 1070Ti_frac]` summing to 1.0.

## 7. Honest expectations

- box B's compute is CPU (SYCL, `-ngl 0`). It already runs a 20.9B MoE at
  25 t/s CPU-only; for 27B dense (all params active) expect slower per-layer.
- Decode = box_A_layers + box_B_layers + 2 x 13 ms network. Projected
  ~8-12 t/s with ~200K+ context capacity (64 GB DDR5 KV) vs current 17.5 t/s
  at 81K.
- Speed loss ~30-50%, context gain ~2-3x. The win is CAPACITY, not speed.
- NPU: not usable for 27B (SYCL backend GPU-only; OpenVINO NPU path limited to
  ~1-3B models, static-graph, stateless). Revisit if a small draft model on NPU
  is ever wanted for speculative decoding.

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