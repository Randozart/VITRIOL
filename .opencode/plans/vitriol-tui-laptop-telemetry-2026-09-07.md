# VITRIOL TUI: Make it see activity on the Intel/PTL laptop (2026-09-07)

## Problem

The VITRIOL TUI shows no activity on Overlord (Arc B390 / no NVIDIA). It was built
for the desktop VITRIOL stack and expects telemetry sources that don't exist here:

| TUI expects | Laptop reality |
|---|---|
| `nvidia-smi` (`nvidia.rs`) | No NVIDIA → empty vec → "nvidia-smi unavailable" |
| `{COPULA_LOG_DIR}/vitriol_gen.log` w/ `slot print_timing` / `[PERF]` / `decode heartbeat` | No file (server via systemd → journald) |
| gen/hermetis/embed/luna/mercury ports (8279/7980/4779/...) | Only flash-next on 8080 |
| shim event stream | None |

## Decisions (confirmed)

1. **Activity (B1)**: tail journald / a log file our server actually writes; reuse
   the existing `slot print_timing` / `parse_decode_speed` parsers (our server emits
   `eval time = ... X tokens per second`).
2. **Intel GPU telemetry**: sysfs primary (no sudo) + `intel_gpu_top -J` fallback
   when installed. Keep `nvidia.rs` untouched; dispatch NVIDIA first, Intel second.
3. **Ports/config**: expose via env (`VITRIOL_GEN_PORT=8080`, `COPULA_LOG_DIR`),
   run wrapper rather than hardcoded defaults.
4. **Desktop regression**: NVIDIA path must keep working (fallback ordering).

## Implementation

### A. `vitriol-tui/src/intel.rs` (new)
- `query_intel_gpu()` → `Option<GpuSnapshot>` from `/sys/class/drm/card0/device/`:
  - name: `card0/device/` (xe drm)
  - vram: `mem_info_vram_used` / `mem_info_vram_total` (in bytes, divide by 2^20)
  - util: `gt/gt0/*` or fdinfo fallback (best-effort)
  - temp: not exposed on xe sysfs → 0
  - power: not exposed → 0
- `query_intel_gpu_top()` → same via `intel_gpu_top -J` when binary present.

### B. `poller.rs`
- In the GPU snapshot step: `nvidia::query_gpus()` → if empty, fall back to
  `intel::query_gpus()`.
- `gen_log()` source: prefer a writable log path; add a journald/`-f` path option.

### C. `ui.rs`
- Replace the literal "nvidia-smi unavailable" empty-state with a generic
  "no GPU telemetry" that still renders when Intel fills the snapshot.
- Ensure GPU card renders when `GpuSnapshot` has name "Intel ...".

### D. Config (`config.rs`)
- Allow `VITRIOL_GEN_PORT` to point at 8080 (already env-driven; ensure no
  hardcoded 8279 in the poller's HTTP probes).

### E. Run wrapper (`scripts/`) or documented env
- `VITRIOL_GEN_PORT=8080 COPULA_LOG_DIR=<dir>` for laptop runs.

## Verification
- `cargo build` + run TUI on laptop → GPU tab shows Arc B390; decode t/s moves
  when a chat completion is running.
- Trigger request via curl; watch decode t/s + log tail update.
- Desktop: NVIDIA path unaffected.

## Out of scope
- Version-controlling the flash-next systemd unit (explicitly declined).
- Making the SYCL server emit `decode heartbeat` (B2) unless B1 proves insufficient.