# Spagyric S4 — Stream-Path Investigation (ternary Qwen) — model file is bad

Date: 2026-08-06.

## 1. Goal

Exercise VITRIOL's `stream` mode on the ternary Qwen (TQ1_0 MoE, the stream-requiring
model), then sweep the VITRIOL knobs (LRU_MB / MAX_LOCKED_MB / PREDICTIVE_PREFETCH /
PIN_FIRST_N_LAYERS / PRUNE_EXPERTS).

## 2. What was unblocked

`sudo /home/randozart/Desktop/Projects/VITRIOL/scripts/vitriol setup` set
`cap_ipc_lock=ep` on llama-server (and llama-cli/bench) + fixed RUNPATH on 39 ELF
files. Binary verified running (`llama-server --version`, ldd resolves libggml-cuda).
Page-locking is now allowed past the 2 GB RLIMIT.

## 3. Stream launch results

- **First attempt** (`VITRIOL_MODE=stream VITRIOL_MAX_LOCKED_MB=4096`): **OOM-killed**
  mid-load. Cause: build has `GGML_VULKAN=ON`, so Chimera mode auto-enabled and routed
  ~1200 small tensors to the VITRIOL VK buffer, whose alloc **mlocks every allocation**
  (`vitriol-vk-buffer.cpp:157`). With CAP_IPC_LOCK now effective, that pins a lot →
  7.4 G model + mlocked VK pool + CUDA buffer > 15 G RAM.
- **Fix: `VITRIOL_CHIMERA_MODE=off`** (llama-model-loader.cpp:1245) → CUDA-only, no VK
  pool. Model then loaded: CUDA0 482 MiB + CUDA_Host VITRIOL buffer 6480 MiB (pageable,
  lazy), healthy after ~80 s, RAM 11 G used / 3.8 G available.
- **BUT output is garbage**: `iams白癜 menda白癜 dicon...`, 1.59 t/s, correct=False.

## 4. Isolation — the model file is the problem, not stream mode

- Same ternary Qwen **native CPU (no VITRIOL, -ngl 0)**: also garbage, tokenizer threw
  `Failed to parse input at pos 0: 白癜 Valu фору...`.
- **Dense BitNet 2B TQ1_0 native CPU (fork)**: **good output** ("It should merge sort
  the data into a sorted data...", correct=True). Fork's TQ1_0 handling is fine.

**Verdict:** `qwen3.6-35b-a3b-instruct-TQ1_0.gguf` (7.4 G) is a bad file — garbage in
every mode (stream and CPU). Suspects: corrupt conversion, wrong TQ1_0 variant/layout,
or a vision-model (`general.tags = ["image-text-to-text"]`) whose tokenizer/vocab the
fork mishandles. The VITRIOL stream path could NOT be faulted: it launched, engaged
(Chimera off), and held a pageable 6.48 G buffer.

## 5. Consequences

- **VITRIOL-knob sweep on the ternary stream path is blocked**: no trustworthy
  stream-requiring model on this box. The 12 G Qwen variants would OOM (11 G avail RAM
  + VITRIOL buffers), and Gemma-4-26B needs ~11 G lock (documented OOM).
- **Deferred items** (hardware/asset gates, not design gates):
  1. A known-good ternary MoE TQ1_0 file (re-download / different conversion).
  2. ≥24 GB RAM box to host a stream-requiring model comfortably.
  3. Then: LRU/prefetch/pin/prune sweep + correctness cross-check against the engine
     reference.

## 6. What DID work (keep)

- CAP_IPC_LOCK + RUNPATH fix: verified, one-shot.
- `VITRIOL_CHIMERA_MODE=off` unlocks stream on CUDA builds with Vulkan compiled in.
- Stream mode engages and holds a 6.48 G pageable buffer within 15 G RAM.
- Fork TQ1_0 handling verified good on the dense BitNet substrate.

## 7. Commands (repro)

```bash
sudo /home/randozart/Desktop/Projects/VITRIOL/scripts/vitriol setup
VITRIOL_MODE=stream VITRIOL_CHIMERA_MODE=off VITRIOL_MAX_LOCKED_MB=2048 \
  ./llama-server -m qwen3.6-35b-a3b-instruct-TQ1_0.gguf -ngl 99 -c 4096 -t 4
```
