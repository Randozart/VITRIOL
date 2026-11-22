# Inkling-2 Mini — why it didn't split across the two cards (and how to make it)

**Research note, 2026-11-22.** Owner question: *"We're splitting Qwen3.8 across two
cards as well — why not with Inkling?"* Short answer: **you can, and you should —
the earlier "blocked on host memory" verdict was a test-configuration artifact, not
an arch or hardware limit.** The Inkling bring-up was run through the wrong recipe.

## What actually works on this box (the 27B proof)

The live 27B daily driver genuinely uses **both** cards. Verified from the running
process (`/proc/<pid>/cmdline`) and `~/.vitriol/config`:

- `~/.vitriol/config`: `tensor_split = 27,21`, `ngl = 99`, `mmap = off`
  (default), `parallel = 2`, `context = 262144`.
- Live cmdline: `vitriol-server -m …Qwen3.8-27B… -ngl 99 … --no-mmap … -ts 27,21`
  — **no `--split-mode` flag**, so it runs the *default* split mode.
- `nvidia-smi` (pre-stop): both GPUs near-full (~1016 MiB / ~837 MiB free) — the
  27B really is distributed across both, not parked on one.

So the mechanism already exists and already works on this host.

## The key facts that clear up the confusion

1. **Default split mode is `LLAMA_SPLIT_MODE_LAYER`** (`common/common.h:485`).
   Both the launcher's `DEFAULT_SPLIT_MODE="layer"` (`scripts/vitriol:275`) and the
   raw binary default to it. LAYER mode distributes **whole tensors/layers**
   per-device by the `tensor_split` ratio — it does **not** need "split buffers".

2. **Only `--split-mode row` needs split buffers.** My earlier `--split-mode row`
   attempt on Inkling failed with `device CUDA0 does not support split buffers`
   (`vitriol-buffer` / CUDA backend has no split-buffer support). That is a
   *row-within-tensor* mechanism, a **different** thing from what the 27B uses.
   LAYER split (what the 27B uses) is fine. I conflated the two.

3. **`--no-mmap` is load-bearing.** The launcher defaults `mmap = off`
   (`scripts/vitriol:343`), i.e. weights are hot-loaded into RAM then split
   across the cards. My bare `llama-cli` bring-up used **default mmap=on**, so
   the weight placement mirrored the whole 17.6 GB onto device 0 (`alloc_tensor_range
   … CUDA0 buffer of size 16631803392` ≈ 94% of the model) instead of honouring the
   ratio. That is the 16.6 GB-on-dev0 I saw — a placement-path artifact of the
   mmap run, not the layer split.

4. **The VITRIOL expert-collapse is off by default.** `VITRIOL_MODE` defaults to
   `VITRIOL_MODE_DISABLED` (`vitriol-cuda-integration.cpp:69`); the host-pinned
   expert routing that "collapses every expert byte onto a single GPU"
   (`vitriol-buffer.cpp:534`) only fires in **STREAM** mode. Neither my tests nor
   the 27B ran in stream mode, so that is not the cause.

## Per-card arithmetic (27:21 split of the 17.6 GB IQ4_NL)

| ctx | dev0 (27/48 wts + ½KV + ~1 GB compute) | dev1 (21/48 wts + ½KV + ~1 GB compute) | fits 12 GB card? |
|---|---|---|---|
| 8 K  | ~11.0 GB | ~8.8 GB | yes |
| 32 K | ~11.3 GB | ~9.1 GB | yes (tight on dev0) |
| 65 K | ~11.7 GB | ~9.5 GB | dev0 over |

So a **modest context (≤ ~32 K)** co-locates fine on 2×12 GB. The 27B's 262 K
window is the exception (its KV is the bulk); Inkling just needs a smaller ctx to
clear the bring-up. (The C5 `probe-recommend` output — "fits ctx 262144 at 39%" —
treats VRAM as one 24 GB pool; that is true for the *pool total* but the per-card
cap still bites at 262 K, which is why the small-ctx split is the safe bring-up.)

## Correct bring-up recipe (replicate the 27B)

```
vitriol-server / llama-cli  -m …/Inkling-2-Mini-IQ4_NL.gguf
    -ngl 99 -ts 27,21 --no-mmap -c 16384 -fa off -n 16 --temp 0
    # default split-mode = layer; VITRIOL_MODE unset (disabled)
```
i.e. the same three ingredients as the 27B (`--no-mmap` + `tensor_split` +
layer split-mode), just a smaller `-c`. This should distribute ~9.9 / ~7.7 GB and
produce coherent text — closing the I6 coherent-text gate that I had marked
blocked.

## Honest caveat (what RESEARCH could NOT verify)

Running a 17.6 GB model load is a state-changing / GPU-memory-consuming command, so
I did **not** execute the successful split run in this research pass. The above is a
code- and arithmetic-grounded prediction. The decisive check (the actual split run
emitting coherent tokens) is a one-command GRIND step. If it still over-allocates on
dev0, the fallback is a heavier dev1 bias (`-ts 20,28`) to keep dev0 under ~10 GB.
