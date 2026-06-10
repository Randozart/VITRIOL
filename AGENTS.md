# VITRIOL AI Agent Guidelines

## Testing Protocol

1. **Always use VITRIOL DMA offloading** for model tests. Never test with `-ngl 0` (CPU-only) unless explicitly requested. VITRIOL is designed to make large models fit on small GPUs — use `VITRIOL_MODE=stream` and `-ngl 99`.

2. **After any build** (`cmake --build`), ask the user to run `sudo vitriol setup` before testing. This sets `CAP_IPC_LOCK` on the server binary, required for page-locked DMA buffers.

3. **Always kill stale servers** with `killall -9 llama-server` before starting a new one.

## Documentation

1. **Write all findings in `.md` reports** with ISO 8601 timestamps (YYYY-MM-DD HH:MM).
2. Place reports in `.opencode/plans/` for integration plans and research findings.
3. Place session logs in `SESSION_LOG_*` and experiment logs in `EXPERIMENT_LOG.md`.
4. Include exact command output, tensor shapes, and error messages in reports.
5. Update the anchored summary in each session with progress, blockers, and decisions.

## Code Conventions

- This is a fork of `ggml-org/llama.cpp` with VITRIOL modifications. The `vitriol` branch contains our changes.
- The VITRIOL predictor is in `ggml/src/ggml-cuda/vitriol-cuda-integration.cpp`.
- The server context checkpoint logic is in `tools/server/server-context.cpp`.
- All VITRIOL env vars are prefixed with `VITRIOL_`.

## Calibration Tool (Rust)

- **`libvitriol/`** — Rust binary for `vitriol calibrate --quick`.
- Build with `cargo build --release` in `libvitriol/`.
- Source files: `gguf.rs` (GGUF v3 parser), `probe.rs` (hardware), `estimator.rs` (VRAM model), `main.rs` (CLI).
- The Rust binary is called by `scripts/vitriol` if built; falls back to Python `libvitriol/gguf_reader.py`.
- **No hardcoded model constants** — all VRAM values computed from GGUF tensor data.
- Self-computing formula: `VRAM = base_model + pin * per_layer_expert + ctx * kv_per_token + scratch + overhead`.
- Overhead heuristic: Pascal=1800, Turing=2200, Ampere=2800, Ada=3200 MiB.
- KV cache computed from model dims: `(embd_len / head_count) * head_count_kv * 2.5 / 1M`.
- Per-layer expert cost from tensor name analysis (`ffn_*_exps` patterns).

## Sweep Controller (Python)

- **`libvitriol/sweep_controller.py`** — automated benchmark sweep via HTTP POST `/completion`
- Starts `llama-server` subprocess per config, benchmarks 64-token generation (1 warmup + 3 measured rounds), reports t/s
- Server readiness: polls `/health` then `/completion` until model fully loads (~15s warmup on GTX 1070 Ti)
- Use: `python3 libvitriol/sweep_controller.py --model <path> --pin 0 8 12 16 --mtp 0 3 5 6`
- Sweep results: 25-config full sweep runs in ~20 minutes; MTP provides zero benefit on this hardware (all scores ~9.7-9.98 t/s)

## MTP (No Benefit on This Hardware)

- Full 5×5 sweep (pin 0/4/8/12/16 × mtp 0/2/4/5/6) completed
- **All configs: 9.6–9.98 t/s**, tightly clustered — MTP has zero measurable effect with Qwen3.6-35B on GTX 1070 Ti
- pin=16 + MTP regresses to 8.58 t/s (VRAM pressure from draft buffers)
- **Optimal: pin=12, mtp=0 or mtp=2, ubatch=128, ctx=65536** → 9.98 t/s
- Full report: `.opencode/plans/mtp-sweep-report-2026-05-25.md`
