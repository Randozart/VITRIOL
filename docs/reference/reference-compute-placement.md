# Reference 3 — compute placement: GPUs, threads, batching

Where the math runs and how work is sliced.

PROVENANCE: arg.cpp semantics; REBIS measurements 2026-08-21/22.

## GPU placement

| flag | meaning | REBIS |
|---|---|---|
| `-ngl N` | layers to VRAM; `99`≈all | both heads fully resident |
| `-ts A,B` | tensor-split fractions across GPUs | unused — co-residency beat streaming (measured: resident IQ2_S beat split IQ3_S 1.7× prefill at equal decode) |
| `-sm none\|layer\|row` | split mode | layer is default |
| `-mg N` | main GPU (intermediate results + KV) | implied by CUDA_VISIBLE_DEVICES usage |
| `-dev LIST` | explicit device list | alternative to CVD env |
| `-fa on\|off\|auto` | flash attention | **on**; required for V-cache quant |

Draft-model variants of all of these exist (`-ngld`, `-devd`,
`--n-cpu-moe-draft`, `-otd` tensor overrides…) for speculative setups
(Reference 5).

## Threads & CPU affinity

`-t/-tb` generation/batch threads · `--threads-http` HTTP workers ·
`--cpu-mask/-range/-strict`, `--prio`, `--poll` (+ full draft-model mirror:
`--spec-draft-*`). Defaults are sensible on this box; we only tune when a
head competes with another workload. Polling trades CPU burn for latency —
leave off for day-long sessions.

## Batching

| flag | default | does |
|---|---|---|
| `-b, --batch-size` | 2048 | logical max tokens per llama_decode call |
| `-ub, --ubatch-size` | 512 | physical micro-batch; prefill processes in ub chunks |

Larger `-ub` = faster big-prompt ingestion but more activation VRAM.
Measured prefill sensitivity to these was minor vs model-placement choices.

## Memory pinning

`--mlock` pins weights in host RAM (prevents swap). We deliberately do NOT
use it: with mmap'd weights the pages stay evictable page-cache, which
survived our RAM-pressure incidents. `--no-mmap` (also not used now) stages
whole weights through anon-RAM — that collision OOM-killed servers during
dual loads on this 15 GB box.

## Research notes

- Placement beats tuning: moving Qwen from dual-GPU streaming to single-GPU
  residency changed prefill 1.7×; no thread/batch knob came close.
- `-ngl 99` on a *fully-offloaded* head means zero CPU involvement in decode;
  partial values (-ngl 30) create hybrid CPU/GPU execution whose speed is
  dominated by PCIe bandwidth — measure before trusting.
