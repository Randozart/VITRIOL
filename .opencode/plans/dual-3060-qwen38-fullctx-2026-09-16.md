# Dual RTX 3060 12GB: Qwen3.8-27B full native 262K + MTP

Date: 2026-09-16 22:35
Status: DONE - daily driver blessed, depth-certified
Owner: randozart

## 0. Hardware change

GTX 1070 Ti 8GB + box B RPC removed. New topology: 2x RTX 3060 12GB (sm_86,
symmetric, 360 GB/s each), host RAM 16->32 GiB. Total VRAM 24 GiB (~23.3
usable; dev0 carries display ~1 GiB).

## 1. Key discovery: qwen35 is a hybrid SSM arch (KV is cheap)

GGUF metadata (extracted via gguf_reader):
- `general.architecture = qwen35`, `qwen35.block_count = 65`
- `qwen35.embedding_length = 5120`, `attention.head_count = 24`,
  `attention.head_count_kv = 4`, `attention.key_length/value_length = 256`
- `qwen35.full_attention_interval = 4` -> only ~17 of 65 layers attend
- `qwen35.ssm.*` present (48 Mamba-style layers, fixed-size state
  `128 x group_count 16 x inner 6144`, does NOT grow with context)
- `qwen35.nextn_predict_layers = 1` (embedded MTP head)
- `qwen35.context_length = 262144`

Consequence: KV cache is ~19.1 KiB/token @ q4_0 (17 attn layers x 4 kv heads x
256 head_dim x 2 x 0.5625 B/elt). Full 262144 window KV = ~5.1 GiB (not ~19
GiB as a dense arch would need).

Model VRAM budget @ 262K, q4_0/q4_0 KV, ts 24,24:
weights 13.2 GiB + KV 5.1 GiB + SSM 0.15 + MTP 0.16 + compute ~1.5 + misc
~0.8 = ~21 GiB. Fits in 23.3 GiB usable.

## 2. Config retarget (Phase 0)

`~/.vitriol/config`:
- `tensor_split = 24,24` (was 0.40,0.30,0.30 3-way RPC)
- `rpc_servers = ` (empty; box B dependency dropped)
- `[spec] type = mtp`, `draft_n_max = 1` (was off at 262K: the old rule
  "262K + MTP does NOT fit" was Pascal-buffer-specific)
- `cache_ram = 2048` (was 256; host RAM doubled)
- context 262144 (unchanged), kv q4_0/q4_0 (unchanged), sparse probe
  (unchanged)

Blessed fingerprint (2026-09-16T22:29:52+02:00):
`model=Qwen3.8-27B-Q3_K_M.gguf ts=24,24 c=262144 kv=q4_0/q4_0 fa=on ub=64
mode=off engine=vitriol-dma ckpt=8192 ckpts=4 lru=0 sparse=sparse
score=probe pool_reset=0 spec=mtp:1 par=1 ssp=1 cram=2048 cis=0`

Diff vs prior blessed (2026-09-06): `ts 24,12->24,24`, `c 81920->262144`,
`cram 256->2048`. spec=mtp:1 unchanged.

## 3. Build fix: upstream renamed mtp -> draft-mtp

The recent upstream merge renamed the spec type: `llama-server --spec-type mtp`
now errors "unknown speculative type: mtp" (binary accepts `draft-mtp`).
Preserved the blessed `spec=mtp:1` identity by restoring the alias in
`llama.cpp/common/speculative.cpp`:
```cpp
// VITRIOL: legacy alias preserved so blessed configs/fingerprints carry
// spec=mtp:1 (upstream merge renamed mtp -> draft-mtp).
{"mtp", COMMON_SPECULATIVE_TYPE_DRAFT_MTP},
```

## 4. Tensor-sharing exploration (vLLM parallelism question)

Question: twin identical sm_86 cards -> use tensor parallelism?

- `-sm row`: DELETED upstream (`74976e1ae "CUDA: remove -sm row, refactor
  cuBLAS (#24216)"`). CUDA backend has no `ggml_backend_split_buffer_type`
  -> "device CUDA0 does not support split buffers".
- `-sm tensor`: the comm/allreduce replacement. EXPERIMENTAL. Crashes in this
  fork: `common_fit_params: llama_params_fit is not implemented for
  SPLIT_MODE_TENSOR, abort` + `GGML_ASSERT(meta_buf_ctx->bufs[i]) failed`
  (ggml-backend-meta.cpp:1760).
- CUDA-graph capture (vLLM's per-token overhead killer): already automatic in
  ggml-cuda (`ggml_cuda_graph_check_compability`, ggml-cuda.cu:2763). Not
  externally configurable.
- vLLM port verdict: NOT applicable. Different engine/kernel/memory model; its
  TP path (NCCL) corresponds to the crashing `-sm tensor`. Layer split + MTP +
  CUDA graphs is the right stack here.

## 5. Benchmarks (3x64, live server /completion, fingerprint-tagged)

| config | decode t/s | notes |
|---|---|---|
| old blessed 3060+1070Ti, c81920, MTP | 18.94 | pre-change baseline |
| dual 3060, c262144, MTP, shallow | 22.85 (22.91/22.84/22.82) | prefill 18.7 |
| dual 3060, c262144, MTP, 245534 filled | 19.54 (19.05/19.05/20.52) | depth cert, GPU util 53/59% |

Depth penalty shallow->245K fill: ~14%.

## 6. Depth certification (Phase 2) - PASSED

`python3 scripts/depth_cert.py --window 262144 --target 245760 --ts 24,24
--kv q4_0 --spec --sample-util`:

```
[3060x2-262k-q4] single-shot 245534 tokens ...
[3060x2-262k-q4] filled 245534
fill_min: 22.7 min
t_s_rounds: [19.05, 19.05, 20.52], t_s_mean: 19.54
oom: false, launchfail: false, gpu_util_mean: [53, 59]
```

Full native 262144 window filled to 245534 tokens, MTP on, q4_0 KV: no OOM.
The old 54K depth wall (Aug-24 cert) is dead. This is the first time full
native ctx + MTP runs locally (was box-B RPC @ ~11 t/s before).

## 7. Bugs found & fixed while certifying

1. `depth_cert.py` used stale flag `--checkpoint-every-n-tokens` (lull-era);
   current build wants `--checkpoint-min-step`. Fixed.
2. `depth_cert.py` `--sample-util` help string broke Python 3.14 argparse
   (bare `%`). Fixed.
3. **MTP draft-context 1 GiB alloc OOMs on empty GPU1 when `--parallel 1` is
   not passed** (exact symptom: `allocating 1024.00 MiB on device 1:
   cudaMalloc failed: out of memory`). The launcher always passes
   `--parallel $PARALLEL`; depth_cert did not. Added `--parallel 1`.
   (Underlying cudaMalloc-on-empty-device behavior not fully explained; the
   pragmatic fix is matching the launcher shape.)

## 8. Standing state

- Daily driver: 262K + MTP n=1, q4_0/q4_0, sparse probe, ts 24,24. GPU0 10.6
  GiB / 1.3 GiB free, GPU1 11.7 GiB / 221 MiB free (knife edge at load; cert
  proves full fill survives).
- ts 24,24 keeps GPU1 tighter than GPU0 (display on dev0). If fill-creep
  resurfaces on GPU1, shift to 25,23. Not needed per cert.

## 9. Follow-ups (not done)

- tq3_0 KV arm at 262K (saves ~1.1 GiB) - unnecessary per cert, available.
- Q4_K_M quant (~16.3 GiB) - quality ceiling at 131-192K.
- libvitriol estimator: KV formula is wrong for hybrid arch (reported 266 MiB
  @ 131K; real ~2.6 GiB). Needs full_attention_interval/head_dim aware calc.
- `calibration/hardware.json` still records the old single-1070Ti probe.
- AGENTS.md Qwen3.8-27B section references superseded 1070Ti-era configs.
- Verify CUDA graph capture actually engaged on the decode graph (no counter
  exposed in /metrics).