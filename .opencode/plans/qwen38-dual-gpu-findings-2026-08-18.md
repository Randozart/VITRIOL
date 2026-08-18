# Qwen3.8-27B on Dual-GPU (RTX 3060 + GTX 1070 Ti) — Full Findings Report

Date: 2026-08-18 18:59
Author: opencode session
Branch: `main` (fork at `61dd39876`, upstream base `b8848`)

## Hardware

| GPU | VRAM | Compute | Role |
|---|---|---|---|
| 0: RTX 3060 | 12 GiB | sm_86 (Ampere) | main-gpu, compute-heavy share |
| 1: GTX 1070 Ti | 8 GiB | sm_61 (Pascal) | secondary share |

Combined usable VRAM: ~19.3 GiB (deducting desktop).

## Model

- `unsloth/Qwen3.8-27B-GGUF` → `Qwen3.8-27B-Q3_K_M.gguf` (13.8 GiB, 4.0 bpw)
- arch `qwen35`, dense, 64 layers, embd=5120, n_head=24, n_kv_head=4
- **Embedded MTP head present** (`nextn_predict_layers=1`), 866 tensors total, 851 loaded for trunk + sibling MTP-head tensors
- Vision encoder NOT supported (fork has no `qwen35vl` — text-only confirmed)

## Build Fix (required)

Original build compiled for `sm_610` only (`ARCHS=610`) because CMake default `CMAKE_CUDA_ARCHITECTURES=native` used the single GPU present at build time. RTX 3060 (sm_86) then failed: `CUDA error: no kernel image is available for execution on the device ... SCALE failed`.

Fix:
```
cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86"
cmake --build build -j8 --target llama-server
sudo vitriol setup   # re-applies cap_ipc_lock=ep after rebuild
```
Confirmed runtime: `CUDA : ARCHS = 610,860`.

## The MTP Bug (found & fixed)

MTP could NOT engage. Root cause chain:
1. Server loads embedded MTP head via `tools/server/server-context.cpp:799` → `override_arch = "qwen35_mtp"`
2. `llm_arch_from_string()` (src/llama-arch.cpp) resolves against `LLM_ARCH_NAMES` string table
3. **Table had no `qwen35_mtp` / `qwen35moe_mtp` entries** despite `LLM_ARCH_QWEN35_MTP` existing in the enum → `unknown override architecture: 'qwen35_mtp'`
4. MTP head load failed → server silently ran plain trunk-only mode (`speculative=none, enabling plain-trunk mode`)

Fix (1-line-pair edit, src/llama-arch.cpp):
```cpp
{ LLM_ARCH_QWEN35,        "qwen35"        },
{ LLM_ARCH_QWEN35_MTP,    "qwen35_mtp"    },   // added
{ LLM_ARCH_QWEN35MOE,     "qwen35moe"     },
{ LLM_ARCH_QWEN35MOE_MTP, "qwen35moe_mtp" },  // added
```
After rebuild: `common_speculative_init: adding speculative implementation 'mtp'` + `set_mtp: MTP draft head registered`.

## KV Cache Quant Levers

KV cache sizing per token (qwen35 hybrid: full attn layers + linear-attn recurrent state):

| ctx | f16/f16 | q4_0/f16 | q4_0/q4_0 |
|---|---|---|---|
| 32K | 2048 MiB | — | ~500 MiB |
| 96K | 6144 MiB | OOM (d1 compute) | 1728 MiB |
| 131K | — | — | 2304 MiB |
| 262K | OOM | — | **4608 MiB ✓** |

- `q4_0 K + q4_0 V` unlocks **full 262K native context** on this VRAM.
- Note: `VITRIOL_KV_QUANT` env does NOT apply; must pass `--cache-type-k q4_0 --cache-type-v q4_0` explicitly.

## Tensor Split

`-ts 24,12` (67/33) is the t/s winner AND fits 262K. Model: 7876 MiB / 4550 MiB. KV @262K: 3168/1440 MiB.
- `-ts 26,10` @262K: RS-cache OOM on GPU0.
- `-ts 22,14`: slower (10.63 t/s) — too much on Pascal.

## Benchmark Results (1 warmup + 3×64-token rounds, greedy)

| Config | t/s | ctx |
|---|---|---|
| ts=24,12, no MTP | 11.11 | 32K |
| ts=22,14, no MTP | 10.63 | 262K |
| ts=24,12, no MTP | 11.03 | 262K |
| **ts=24,12, MTP n=5** | **14.08** | 32K |
| **ts=24,12, MTP n=5** | **14.09** | **131K** ← WINNER |
| ts=24,12, MTP n=8 | 14.04 | 32K |
| ts=24,12, MTP n=10 | 14.07 | 32K |
| ts=24,12, MTP n=5, real reasoning | 13.04 | 131K |

MTP plateau at ~14.05 t/s across n=5/8/10 → verify-bound, not draft-depth-bound.

## MTP Findings

- **MTP = +28%** (11.03 → 14.09 t/s). Contradicts prior AGENTS note "MTP zero benefit" — that was measured on MoE Qwen3.6-35B AND the MTP path was silently broken. On dense Qwen3.8 the embedded head is excellent.
- **100% draft acceptance** on both greedy and real reasoning generation (74/74, 108/108). Embedded MTP head shares trunk weights — no separate draft model needed.
- **VRAM cost:** MTP head + draft context needs ~126 MiB on GPU1. Fits at ≤131K, OOMs at 262K (Pascal compute buffers saturate).
- Draft depth n=5 is sufficient; higher depths give nothing.

## Final Recommended Config

```
CUDA_VISIBLE_DEVICES=0,1 VITRIOL_MODE=stream VITRIOL_KV_MODE=standard \
./build/bin/llama-server -m Qwen3.8-27B-Q3_K_M.gguf \
  -ngl 99 -c 131072 -ts 24,12 --main-gpu 0 -ub 128 -np 1 \
  --cache-type-k q4_0 --cache-type-v q4_0 \
  --spec-type mtp --spec-draft-n-max 5
```
→ **14.09 t/s @ 131K ctx** (2 GPUs, all on-device).

### Tradeoff decision
- Need 262K (native max) → drop `--spec-type mtp` → 11.03 t/s.
- Want max speed → keep MTP @ 131K → 14.09 t/s.

## Side Notes

- Sweep controller (`libvitriol/sweep_controller.py`) lacks `-ts` (added during this session) and its MTP path never engaged MTP — results invalid as MTP tests, valid as plain-mode baseline.
- Sweep OOM exits segfault (kernel-alloc failure path) — cosmetic.
- The `--output` CSV arg in sweep controller is not implemented.
- scripts/vitriol banner hardcodes "GTX 1070 Ti" (cosmetic, stale).

## Commits/Edits Made

- `src/llama-arch.cpp`: added `qwen35_mtp`, `qwen35moe_mtp` string entries (MTP fix).
- `libvitriol/sweep_controller.py`: added `--ts` tensor-split sweep support.
- No upstream cherry-picks performed — the missing MTP path was a local fork bug, not an upstream gap.
