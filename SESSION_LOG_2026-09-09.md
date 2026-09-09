# Session Log — 2026-09-09

Anchored summary of the day: VITRIOL transcends one machine (distributed
row-split inference via ggml-rpc), plus recording of the work. Full details in
the linked docs/plans; this file is the index + outcome record.

## Distributed inference — row-split over RPC (DONE, working)

- **Goal**: extend capacity (context/KV) by offloading whole layers to a
  second machine. Chosen: llama.cpp RPC backend (row-split), not
  tensor-parallel (network allreduce latency wash) and not WebRTC (wrong
  transport for ordered zero-loss tensor handoff on a mesh).
- **Transport**: raw TCP over the Tailscale mesh (13 ms RTT, no public
  exposure). No WebRTC, no NAT traversal.
- **Setup**: both ends built with `-DGGML_RPC=ON`; op-count fix
  (`GGML_OP_COUNT` 101->102) + RPC protocol patch 0->1 so both sides negotiate
  v6.0.1 (llama.cpp `a6bf4a2fe`). Box B (overlord-x8664, Intel Core Ultra 16C,
  64 GB DDR5) runs `ggml-rpc-server -H 0.0.0.0 -p 50052` as a user systemd
  unit (`scripts/systemd/vitriol-rpc-server.service`, enabled, Linger=yes).
- **Measured** (box B CPU, 13 ms mesh): local 18.94 t/s decode / 34 prefill;
  RPC 5% box B 11.35 / 7.66; 25% 10.03 / 6.03; 40% 4.87 / 4.87.
  - Decode sweet spot ~5% slice. Prefill is box B DDR5-bandwidth-bound (streams
    ~3 GB slice per token) — the honest wall. 20 t/s prefill **judged
    acceptable** (owner decision).
  - Win is **capacity**: box B's 64 GB RAM extends context/KV beyond box A's
    VRAM depth wall. Speed loss ~45% at 5% slice.
- **Launcher plumbing** (outer `5f1e932`): `[gpu] rpc_servers` config key ->
  `--rpc` in all 4 launch sites; `rpc=` in fingerprint (serve + bless paths);
  config status/show surfaces it; bundled `profiles/qwen38-distributed/`
  (5% split + rpc_servers).
- **Parked**: prefill-on-A / ship-KV via `SET_TENSOR` (real option if 20 t/s
  becomes a blocker); CUDA Rust (CC 8.0+ floor excludes the 1070 Ti / Intel
  box B — future direction only).
- Recorded: `docs/DISTRIBUTED_INFERENCE.md` (canonical), README paragraph,
  CONFIG_REFERENCE `rpc_servers` section. Plan: `.opencode/plans/
  distributed-inference-row-split-2026-09-08.md`.

## Also this session

- **Chunked GDN kernel** (llama.cpp `8382994a0`, pushed earlier): E1-E7 chunk
  prefill for the MTP-verify serial wall. Bit-identical full-model A/B,
  17.43 -> 17.54 t/s (parity; wins at C>=6 draft chains this box rejects).
- **SwarmLLM mining arc closed** (`75a3247`): Phase 3 candidates assessed —
  4.7 not a CUDA bug (vocab dim is grid.x, not grid.y), 4.5/4.4 deferred.

## Commits this day

- llama.cpp: `8382994a0` (chunked GDN), `a6bf4a2fe` (RPC protocol patch 1).
- outer: `f93e16f` (distributed setup doc), `5f1e932` (launcher + profile +
  systemd unit), docs-recording commit (this session's docs).

## Decisions

1. Accept 20 t/s prefill on box B — do NOT build prefill-on-A/ship-KV now.
2. CUDA Rust deferred (CC 8.0+ floor; NVIDIA-only; box B is Intel/SYCL).
3. Row-split over RPC is the distributed path; TP and WebRTC rejected.
4. Recording done with host IPs anonymized (mesh IPs are operator-specific).