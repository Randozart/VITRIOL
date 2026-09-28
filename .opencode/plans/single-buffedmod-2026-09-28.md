# Single-Session Profile: GSQ-RCO BuffedMod Q6K, 208K ctx, MTP n=3

- **Date:** 2026-09-28 19:07
- **Status:** DONE (live daily driver)
- **Supersedes:** `duo-393k-ud-iq3s-2026-09-17.md` (ABANDONED — see
  `slot-scaling-2026-09-28.md`: 2 slots give ~1.0x throughput, not a win)
- **Related:** `probe-capture-leak-2026-09-28.md` (the `pre_decode` crash fixed
  the same day; this cert exercises that path under the new config)

## Goal

Replace the duo (2 slots / 393K shared pool) with a single-session daily driver
and take a slightly-bigger quant of Qwen3.8-27B, keeping the GSQ-RCO recipe.

## Model selection

RCO ceiling is 3.50 bpw (IQ3_S). ISTA ships only IQ2_XS/IQ2_S/IQ3_XXS/IQ3_S;
every derivative repo (ukisai, Heretic, npario/taurusduan/rAVEUK mirrors,
RentedNoodle, Blackfrost) also stops at 3.5 bpw. No RCO 4-bit exists, and RCO
IQ3_S is already **task-lossless** (ISTA evals: AIME25 100.00, LCB v6 85.71,
GPQA-D 89.39 vs BF16 89.90; task avg 91.70 = 99.8% of 91.87), so a 4-bit RCO
buys ~nothing. The only true 4-bit options are non-RCO (unsloth UD-IQ4_XS
4.22 bpw) which requires dropping the RCO recipe — rejected.

Chosen: **`tooltd/Qwen3.8-27B-GSQ-RCO-BuffedMod-GGUF`**
`Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf` (12,447,799,360 B,
11.59 GiB). Direct tensor surgery on the ISTA IQ3_S base: `output.weight`
(LM head) Q4_K -> Q6_K. Publisher llama-perplexity vs BF16:
PPL-diff error 0.1122 -> 0.0817 (-27.2%), median KLD 0.0229 -> 0.0189
(-17.5%), top-token match 89.66% -> 90.54%.

Verified on download:
- sha256 `dc3cd01b60ca6e535793c331e7e5a24096e5841ba546574afe866542fa8df8e7`
  == tree-API `lfs.oid`.
- Tensor-type diff vs ISTA IQ3_S-mtp: **1 tensor** changed (`output.weight`
  Q4_K -> Q6_K); metadata identical; arch `qwen35`, `nextn_predict_layers=1`.
- The IQ4XS variant (sha256 `e273548f…`) was also fetched/validated (diff =
  output.weight + 8 `blk.64` nextn tensors) but not chosen; deleted after cert
  to reclaim ~12 GB.

## A/B (single slot, ts 24,24, q4_0/q4_0, 42.7k filled, 128 tokens)

| arm | decode t/s | MTP accept | dev1 free |
|---|---|---|---|
| ISTA IQ3_S n=1 (control) | 21.25 | 0.868 | 267 MiB |
| Q6K n=1 | 21.09 | 0.868 | 179 MiB |
| IQ4XS n=1 | 20.84 | 0.868 | 61 MiB |
| **Q6K n=2** | **23.72** | 0.778 | 107 MiB |
| **Q6K n=3** | **24.3-24.9** | 0.717 | 35 MiB |
| Q6K n=4 | load crash | - | - |
| Q6K n=3, ts 25,23 | dev0 OOM crash | - | - |

Verdict: Q6K is the faster/quality upgrade; MTP `draft_n_max=3` is the fastest
that loads. This supersedes the old "n_max MUST be 1" rule (that was
Q3_K_M/Pascal-era; measured here with the Q6K MTP head).

## VRAM wall and the 208K context

The live window was `-c 262144`, not ~393K (duo never went live). With the
other GPU services on this box (lares-vision ~1.85 GiB dev0, victim-id ~0.49
GiB dev1, desktop), 240K + MTP n=3 does not load at all. Load matrix
(with GPU settle between runs, Q6K, MTP n=3, f16 draft KV):

| ctx | result | dev1 free |
|---|---|---|
| 196608 (192K) | stable | 534 MiB |
| **212992 (208K)** | **stable, no retry** | **230 MiB** |
| 229376 (224K) | loads w/ compute-buffer retry | 130 MiB |
| 245760 (240K) | load OOM | - |

Chosen **212992 (208K)**: largest context with a stable, retry-free load.
A conservative **`qwen38-buffedmod-192k`** profile (196608) is bundled as a
fallback for when external GPU memory grows.

### Draft KV quantization: negative result

Added launcher support for `--spec-draft-type-k/v` (`[spec] draft_type_k/v`,
`dkv=` in the fingerprint) hoping to shrink the MTP head's f16 KV (~960 MiB at
240K). **Counterproductive:** q4_0 draft KV breaks flash-attn for the MTP layer
and its compute pp buffers balloon from ~300 MiB to ~1.2 GiB on dev1 -> load
OOM. Left empty (f16). The knob is kept for future use.

## Live config (fingerprint)

```
VITRIOL-FINGERPRINT model=Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf \
  ts=24,24 c=212992 kv=q4_0/q4_0 fa=on ub=128 mode=off engine=vitriol-dma \
  ckpt=8192 ckpts=4 lru=0 sparse=sparse score=probe pool_reset=0 spec=mtp:3 \
  par=1 ssp=1 cram=12288 ct=chat-template-tolerant.j2 cis=0
```

Deltas vs previous blessed arm: `model=`, `c=262144->212992`, `spec=mtp:1->mtp:3`,
`par=2->1`.

## Certification

Restarted via unit; per-GPU VRAM dev0 10933/1028 free, dev1 11733/230 free.

| arm | filled | decode t/s | notes |
|---|---|---|---|
| shallow | 21 | 14.1-15 | MTP warm-up noise |
| 27K | 42,678 | 25.26 / 25.40 | prefill 385 t/s, accept 0.705 |
| deep | 116,808 | 19.79 / 20.32 | prefill 238 t/s, accept 0.958, **probe armed** |

`VITRIOL_KV_SCORE: probe active, n_kv=159744 layers=1` — the MTP draft context
was the scored memory at depth, i.e. the exact cross-context path of the
`probe-capture-leak` fix ran clean. Zero `pre_decode() failed`,
`unordered_map::at`, `cudaMalloc failed`, or OOM lines. Warm restart restored 4
prompt states and re-served with no errors. Only error is the pre-existing
missing `slot0.bin` boot warning.

## Artifacts

- Models: `~/Models/Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf` (live);
  ISTA `Qwen3.8-27B-GSQ-RCO-IQ3_S-mtp.gguf` retained for rollback.
- Profiles: `~/.vitriol/profiles/qwen38-buffedmod-208k` (live) +
  `qwen38-buffedmod-192k` (conservative); bundled in `profiles/`.
- Launcher: `scripts/vitriol` gained `[spec] draft_type_k/v` -> `dkv=`.
- Config backup: `~/.vitriol/config.bak` (pre-single).

## Rollback

`vitriol config load qwen38-buffedmod-192k` for more headroom, or restore the
ISTA model path in `~/.vitriol/config` and restart the unit.
