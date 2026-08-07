# Persistent KV activation cache + tokenization — research record

Date: 2026-08-07.

## 1. Idea (user, 2026-08-07)

1. **Tokenization**: pre-tokenize repo files AOT and store `u32` token-ID arrays in
   Pymander; compile the BPE vocabulary into a static trie/FSM; parallelize over
   regex word boundaries (rayon). Claimed bottleneck: regex pre-tokenization,
   greedy merge O(N²), heap thrashing.
2. **Persistent out-of-core KV cache (Hermetic Activation Cache)**: run a
   background prefill over static files AOT, serialize the per-layer Key/Value
   tensors to disk keyed by the file's BLAKE3 hash, and on load inject the cached
   KV directly into the context — bypassing prefill ("zero prefill latency").
   RoPE position shift handled by an in-register rotation shader.

## 2. External validation

- **fastokens v2** (NVIDIA & Crusoe, 2026): regex is the real BPE bottleneck;
  zero-alloc Unicode scanner + thread-local caches → 9.1× over HF, 40% TTFT cut.
- **BlockBPE** (Dec 2025): BPE merge graph compiled to GPU thread blocks, 2.5×.
- **Trie/FSM BPE**: greedy merge ≡ longest-prefix walk on a trie, O(N) linear.
- **vLLM / SGLang prefix caching**: hash blocks; shared prefix skips tokenize +
  prefill → up to ~90% input-token cost cut. Our idea = same technique applied
  to static files, persisted across sessions.

## 3. Analysis (2026-08-07)

### Tokenization
Real research, **low value for VITRIOL**: naive BPE ≈ 1 MB/s+; a 100 KB codebase
is ~0.1 s. The felt multi-second pause when loading a repo is **prefill**, not
tokenization. Swap to a trie FSM saves ms; killing prefill saves s. **Parked.**

### Persistent KV cache — mathematically sound
- Re-using cached KV of a static block is exactly prefix caching.
- **RoPE re-rotation is exact and simpler than feared**: K at position `p` is
  `R(p·θ)·K_proj`; shifting the whole block by `Δ` = `R(Δ)·K_cached` — one
  constant rotation matrix applied to every cached K at every layer (θ base is
  layer-independent in deepseek). **V is never RoPE-rotated — untouched.**
  Block-internal relative positions unchanged → internal attention valid.
- Requires the same model (weights/quant) — key the cache by model + file hash.

### Honest caveats (the plan glossed these)
1. **"Index once" is a full prefill** — amortized, but a real win only if the
   same files are injected repeatedly. File edits invalidate via hash. llama.cpp
   already has in-session `cache_prompt`/`--cache-reuse`; the incremental value
   is **cross-session** persistence.
2. **KV is gigabytes**: DeepSeek2-Lite ≈ 30–115 KB/token K+V; a 30 K-token file
   ≈ 1–3 GB. On 8 GB VRAM (model ≈ 5.5 GB) the cache lives in host RAM and
   streams over PCIe 3.0 (~12 GB/s) — not a "microsecond copy".
3. **Prefill win, not decode win**: every decode step attends over the *entire*
   cached block. On Pascal, decode-phase attention over a 30 K-token cache may
   cost more than the prefill it saved — **a measurable, possibly refuted
   hypothesis on this silicon.**

## 4. Recommended path (Golden Rule 7: measure before build)

1. **Measure first**: capture per-layer K/V for a file in the fork (same
   machinery as the expert-fired hook), store to disk, and benchmark
   prefill-only vs load-cache + decode-attention at cache sizes 1K/4K/16K
   tokens on this exact binary. If the cache is not a net win at the sizes
   actually injected, the hypothesis is refuted and the build is blocked.
2. If it survives: fork surgery = capture K/V → serialize (BLAKE3-keyed) →
   inject into a `llama_context` + the uniform-Δ RoPE-rotation shader. Weeks of
   C++/CUDA in the fork.
3. Tokenization hacks parked unless measurement shows they matter.

## 5. Status

Recorded. Not started. Parked behind the measurement gate + the Hermetis port.
