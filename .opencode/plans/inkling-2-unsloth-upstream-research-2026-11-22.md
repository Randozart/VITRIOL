# Inkling-2-Mini: Unsloth + upstream llama.cpp — what we were missing

**Date:** 2026-11-22
**Mode:** RESEARCH (no state changed; engine left `active`, `/health ok`, GPUs 10930/10110 MiB)
**Trigger:** owner supplied https://unsloth.ai/docs/models/inkling-2-mini
**Context:** our frozen fork (`engine/`, upstream sync FROZEN 2026-10-23) has Inkling-2-Mini
porting to ~15.1 t/s decode, 7,926 kernels/token, GPU0 36 % busy. The open question was
whether the remaining speedup required a fused-CUDA-kernel project that would trade away
bit-exactness. **This research says: no — the biggest win needs no engine code at all.**

---

## 1. The headline finding: DSpark speculative decoding is already in our fork

Upstream and Unsloth both ship Inkling-2-Mini with **DSpark** speculative decoding, and the
measured gain is **+68 % decode**, not the single-digit percent I was chasing with graph
tweaks.

- Unsloth's documented launch line
  (https://unsloth.ai/docs/models/inkling-2-mini, L268–275) is model + mmproj +
  `--model-draft <dspark gguf> --spec-type draft-dspark --spec-draft-n-max 2
  --n-gpu-layers-draft 99`. They also list a "DSpark drafter 0.4 GB Q8_0" download
  (L297) and say **n_max 2 is fastest** for this model.
- The drafter is `unsloth/Inkling-2-Mini-GGUF` → `dspark-Inkling-2-Mini-Q8_0.gguf`,
  **402 MB**, block size 8 (mainline-format variants live under `dspark/`).
- DSpark = DFlash block-diffusion drafting + a low-rank previous-token **Markov head** that
  biases each block position's logits, chained in-graph (semi-autoregressive; a whole block
  per forward pass). Upstream PR **#25173** "spec: add DSpark speculative decoding", merged
  2026-07-28 (`84075273c`); `{"draft-dspark", …}` at `common/speculative.cpp:78`.
- Measured in PR **#32773** (16 GB RX 9070 XT, 184k ctx): code **47.6 → 80.2 tok/s (+68 %)**
  at n_max 4, 74.7 at n_max 2. **French prose regresses** 47.7 → 39.5. Default f16 draft KV
  collapsed to 22.7 tok/s — **the draft KV must be q4_0**.

### Verified present in OUR frozen fork — zero code needed

| piece | location in `engine/` |
|---|---|
| `draft-dspark` spec type | `common/speculative.cpp:46` |
| DSpark impl | `common/speculative.cpp:968` ("the draft carries a Markov head and uses an anchor-first block layout"), `:2380` |
| variant detection | `src/models/dflash.cpp:114` — `ml->get_tensor_meta("markov_w1.weight")` |
| Markov tensors | `src/llama-arch.h:744-746` (`DSPARK_MARKOV_W1/W2`, `DSPARK_CONF_PROJ`) |
| markov head graph | `src/models/dflash.cpp:296,367,819,998` |

So the +68 % path is: **download the 402 MB drafter, add three flags.** No engine change, no
numerics trade, reversible by removing the flags.

## 2. Why my previous plan was aiming at the wrong target

I had concluded that the remaining 18 % (`clip()`), 15 % (`control()`) and 9 % (aux matvecs)
were only reachable via new fused CUDA kernels that would break bit-exactness engine-wide,
and that graph-builder changes capped out around 19–21 t/s. That analysis was correct *for
the decode path as measured* — but it optimised the wrong variable. DSpark does not make the
per-token forward pass faster; it **amortises it over ~2–4 tokens per pass**. At 15.1 t/s
with a ~1.9 acceptance rate, the effective decode is ~28–34 t/s. That is a bigger win than
every kernel fusion combined, for zero correctness surface.

The `clip`/`control` work remains valid as a follow-up (it also helps the draft-verification
pass), but it is no longer the critical path.

## 3. Upstream has open perf PRs for exactly the defects we found

All of the following are **open and unmerged** as of 2026-11-22, i.e. upstream has not
solved them either:

| PR | technique | quoted effect | applicable here? |
|---|---|---|---|
| **#32740** | `ggml_flash_attn_ext_add_indexed_bias()` — bias supplied as `table F32 [n_bins,n_head,n_tokens]` + `indices I32 [n_kv,n_tokens]`, CPU adds `mv += bias[bins[ic]]`; **CUDA declines** | 33k prefill **301→1,036 tok/s**, −415 MiB VRAM; decode regresses 67→58 | partially — this is the API contract for #32808 |
| **#32752** | chunked mask build, 32-row `ggml_set_rows` chunks (`llama-graph.cpp`) | compute buffer 90k ctx **8.69→3.19 GiB**, pp@64k 377→384 | **yes** — memory, not speed |
| **#32777** | `llama_model_layer_visits()` ranks looped MoE layers first in `--fit` | compute buffer **7,721→983 MiB**, pp512 95→245 tok/s | **no** — our fork has no `--fit` / `layer_visits` (verified absent) |
| **#32808** | CUDA `fattn_indexed_bias::add()` + `use_rows`/`indexed_bias` templates = **kv_rows with a per-head mask** | **9.5×–54.7× layer speedup**, 14× less RAM | yes in principle; large |
| **#32855** | swap `mix()` operands | prefill **+14–37 %** | **yes** — 2 lines, prefill only |
| **#32751** | `-sm tensor`, split r-state / mirrored s-state | KLD 0.0028 | conditional |

**#32808 is the fix for our depth collapse** (16.0 t/s at 1k → 5.5 t/s at 8k). Root cause is
confirmed structurally in our own tree: `llama-graph.cpp:2670` sets
`use_flash_attn = cparams.flash_attn && kq_b == nullptr`, so **any per-head relative bias
forces flash attention off**, and `:2744` then does a dense
`kq = ggml_add(kq, kq_b)` on an `[n_head, n_kv, n_tokens]` tensor. `#32808` is precisely
"give flash attention a per-head mask". Upstream's own code comment carries the same TODO
("needs a kv_rows flash attention with a per-head mask (CUDA has none)") — so this is a real,
acknowledged gap, not our porting mistake.

## 4. Correctness bugs we may have inherited

| issue | state | relevance to us |
|---|---|---|
| **#32710** PPL NaN on CUDA | open; **fixed in code** by **#32807** (`5bacb7c86`, 2026-11-22) | Root cause: only one device copy of a host expert weight is made and added as input of the *first* split, so **loop passes 2–3 run `MUL_MAT_ID` on stale expert data**. **Not applicable verbatim**: our fork has no `sched_copy_experts` (verified absent). Worth a cheap diagnostic all the same — upstream users report **`--no-op-offload` fixes** the garbled-output family. |
| **#32778** all-`?` / garbled past ~96 tokens | open; grammar-crash side-effect fixed by #32833 | Same family as #32710. Reporter still suspects **shortconv state + prefix-cache propagation**. This is the closest upstream analogue to the kind of corruption we chased with the strided-SCALE bug. |
| **#32785** KV locality | open, 0 comments | `llama_kv_cache` placement follows `-ts` only, ignoring `-ot`; 48 unrolled layers → **445–509 graph splits**. Directly relevant: we run `-ts 27,21` with 47 layer slots, so our split count is inflated the same way. |
| **#32707 / #32846** slow prefill | open | Not correctness: **materialised F32 rel-pos bias** (`k_get_rows_float_vec` 18–20 % GPU) + scalar copies (~14 %). Same mechanism as our depth collapse. |

## 5. Quantization: no reason to change

Measured on this exact hardware class (RTX 3060, sm_86, `test-backend-ops perf -o MUL_MAT
-b CUDA0 -p "n=1,"`, GEMV m=4096 k=14336):

| type | µs | GB/s |
|---|---:|---:|
| iq4_xs | 98.2 | 324 |
| **iq4_nl** | **105.1** | **314** |
| q4_K | 109.1 | 303 |
| q4_K (baseline) | 109.1 | 1.00× |
| q8_0 | 191.2 | 326 |
| f16 | 357.4 | 329 |

- All 4-bit formats hit 300–325 GB/s (~90 % of 360 GB/s peak). **IQ4_NL suffers no sm_86
  decode penalty** (it is 4 % *faster* than Q4_K here).
- Unsloth's recommended file is Q4_K_XL at **17.63 GB**; our IQ4_NL is **17.63 GB**. Same
  footprint, same speed class → **no reason to re-download**.
- Kernel paths: `mmvf.cu:792-879` (mmvf = F32/F16/BF16 only) vs `mmvq.cu:1510-1525`
  (`quantize_row_q8_1_cuda` extra launch). So the aux-as-F32 idea *would* skip the
  `quantize_q8_1` partner — confirming the mechanism, but the payoff is latency-shaped, not
  bandwidth-shaped, and is now deprioritised behind DSpark.

---

## What I recommend, in order

1. **DSpark drafter** (402 MB) + `--spec-type draft-dspark --spec-draft-n-max 2
   --n-gpu-layers-draft 99`, draft KV **q4_0**. Zero engine code, measured +68 % on the
   comparable hardware class, fully reversible. **This is the answer to "beat Qwen3.8-27B in
   speed".**
2. **#32855 `mix()` operand swap** — 2 lines, +14–37 % prefill, no numerics trade.
3. **Re-test with `--no-op-offload`** as a cheap diagnostic for the garbled-output family
   (#32710/#32778), given we run 2 splits over 47 slots.
4. **#32752 chunked mask** — compute-buffer memory at long ctx.
5. **#32808 kv_rows flash attention** (`use_rows`/`indexed_bias` templates) — the real fix for
   the 8k depth collapse; large, follow-up.
6. Deferred: fused `clip`/`control` kernels (previous plan) — still valid, no longer critical.

## Invariants worth pinning (for the PLAN phase)

- **I-A1** 27B `probe-recommend` byte-identical; live engine still serving Lapis Occultus.
- **I-A2** DSpark disabled ⇒ decode signature `03c0150ddd05` unchanged (proves the flags are
  the only delta).
- **I-A3** DSpark enabled ⇒ output coherent and matching the non-speculative text on a fixed
  prompt (speculative decoding must not change the distribution, only the speed).
- **I-A4** Draft KV explicitly q4_0 (f16 is a measured 3× regression).
- **I-A5** Every report carries the `VITRIOL-FINGERPRINT:` line including the new spec flags.
