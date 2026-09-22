# Vendor Audit + GSQ-RCO Qualification: saas-home/Qwen3.8-27B-NV-16GB

Date: 2026-09-22
Status: DONE
Owner: randozart

## 1. Provenance / provenance of the nick

Audited `saas-home/Qwen3.8-27B-NV-16GB` (fork of MiaAI-Lab Qwen3.8-27B-16gb
one-click install; ExLlamaV3 serving kit; MIT License (c) 2026 Mia's AI Lab).

**Pulled in:**
- `tests/scripts/` long-context qualification harness (9 scripts, MIT) →
  vendored to VITRIOL `tests/scripts/` with PROVENANCE headers + MIT notice
  retained (commit `d16ac10`). Adaptations: llama-server endpoint default
  (port 8279), `VITRIOL-FINGERPRINT` embedding in stdout + all JSON results,
  `--blocks` depth scaling + proportional stage placement on the stress
  tests, vision stage dropped (text-only engine).
- **GSQ-RCO IQ3_S GGUF** (ISTA-DASLab, 11.29 GiB, `-mtp` variant with
  embedded MTP head): fetched via ranged-resume (HF CDN stall doctrine),
  sha256 verified `58fd8267...` == tree-API lfs.oid.

**Rejected / already-owned (audit table):**
- Reasoning hygiene (`--no-reasoning-preserve`, `--reasoning-budget`,
  `reasoning_effort`) — already in our engine
  (`common/reasoning-budget.cpp`).
- Host prompt cache tier (CPU_CACHE_GB=24 DDR5) — host is 32G DDR3 + Bachi
  + OOM doctrine; our cache-ram/slot-ckpt already cover it, trimmed.
- CCD0 CPU affinity — i7-3770, single socket.
- PARALLEL=2 BatchWorker — llama-server parallel is native; we run par=1
  by VRAM choice.
- EXL3 3.0bpw / 204k horizon — Ada-tuned kernels; not transferable to
  our engine/stack.
- is_headless()/expandable_segments/MAX_TOKENS clamp — PyTorch-land or
  already-native.
- **Asymmetric KV (4-bit K / 3-bit V)**: the idea was already tested and
  CLOSED on our engine (2026-09-17 sweep, EXPERIMENT_LOG):
  `tq3_0 K + q4_0 V` = -27.7% (Hadamard dequant overhead); `tq3_0 V` =
  KNOWN BROKEN on CUDA. Their win comes from ExLlamaV3 fused bit-plane
  kernels, not transferable. Recorded as closed, not re-attempted.

## 2. Harness baseline (dual RTX 3060, reduced-ctx geometry)

Bachi train loop (`full64_stream.py`, GPU0, 3.4 GiB, respawns ~9.5 min) +
lares-llama supervisor made full-262K cert impossible; user chose
reduced-ctx. Geometry for ALL arms below:
`c=98304, ts=14,34, ub=64, kv=q4_0/q4_0, fa=on, spec=mtp:1, ckpts=2,
cache-ram=512, parallel=1, port 8299, bare llama-server (unit inactive,
inner-fork build ab61c2a01, no lull-kv)`. Fingerprint in every log/result.

Ladder (depth_cert.py single-shot fill + 3x64 decode):

| depth filled | Q3_K_M (t/s) | GSQ-RCO IQ3_S-mtp (t/s) | delta |
|---|---|---|---|
| 8k | 16.39 | 25.68 | +57% |
| 32k | 18.27 | 26.26 | +44% |
| 64k | 16.72 | 26.16 | +56% |
| 90k | 17.53 | 26.16 | +49% |

GSQ-RCO decode is ~50% faster and essentially FLAT across depth (25.7→26.2);
Q3_K_M dips at 64k. Weight traffic alone (-12%) does not explain +50% —
RCO tensor layout + embedded native MTP head are the likely contributors.
Caveat: ub=64 + skewed ts, not blessed ub=128; internal A/B is fair because
geometry is identical. Absolute numbers NOT comparable to the 245K cert
(blessed geometry, empty GPUs).

## 3. Accuracy arms (24k depth, sparse substrate OFF by engine reality)

High-entropy KV recall (25 pairs / 8 targets):

| model | thinking | recall |
|---|---|---|
| Q3_K_M | default (server ON) | 8/8 PASS |
| GSQ-RCO | default (server ON) | 0/8 FAIL (artifact) |
| GSQ-RCO | `--no-thinking` | 8/8 PASS |

The 0/8 was a TEST ARTIFACT: server defaults thinking ON for the Qwen3
template; GSQ-RCO spent the whole 1000-token budget in `reasoning_content`
(verbatim correct retrieval visible there) and never emitted `content`.
Q3_K_M happened to finish thinking in budget. Harness now has
`--no-thinking` (forces `enable_thinking: False`). With thinking OFF both
models recall 8/8 identically.

Extreme-precision (14-stage ALU + 25-tx ledger), both models: PASS / PASS.
Multi-hop 8-hop dependency: FAIL on BOTH models identically — the model
meticulously computes intermediate XORs in verbose reasoning and exhausts
the 1500-token budget before reaching `var_h` (verified in raw probe).
Test-design artifact, not quant damage; not discriminative for these
large-thinking models.

## 4. Conclusions / recommendations

1. **GSQ-RCO-IQ3_S-mtp is a strict win on this rig**: +50% decode, flat
   depth curve, equal accuracy on all discriminative stress tests. Weight
   11.29 GiB vs Q3_K_M 12.87 → also more headroom for the duo-393K plan.
   RECOMMEND: consider `vitriol config bless` retarget to GSQ-RCO when the
   daily unit returns to full-262K geometry (fingerprint diff mandatory,
   full-depth cert at blessed ub=128 before cutover).
2. **Harness improved**: `--no-thinking` flag (proven necessary), `--blocks`
   depth scaling, fingerprint embedding. Keep.
3. **Multi-hop stress test is not discriminative** for reasoning models —
   note in tests/scripts/README; consider a constrained-reasoning variant
   later (e.g. force concise final-answer-only output).
4. **lull-kv sparse substrate is absent from the current inner-fork build**
   (`VITRIOL_KV_MODE` not in binary/libllama; launcher guard would refuse a
   sparse launch). Daily driver has been effectively running non-sparse.
   Stage-6 port remains open.
5. Foreign-resource note: lares-llama.service is a supervised systemd --user
   unit (respawned after kill — stopped via `systemctl --user stop
   lares-llama.service`); Bachi `full64_stream.py` loop is untouchable
   (user-declared important). Cert geometry must tolerate it.

## 5. Files

- Vendored harness: `tests/scripts/` (commit d16ac10)
- `scripts/depth_cert.py`: added `--ckpts`/`--cache-ram` overrides,
  fingerprint carries `ckpts=`/`cram=` (doctrine)
- Model: `~/Downloads/Qwen3.8-27B-GSQ-RCO-IQ3_S-mtp.gguf`
  (sha256 58fd826723939933dc86f45b7fe04545cbc2de1c70f6fe2cdd3858c87a98c12f)
- Results: `/tmp/opencode/{high-entropy-baseline,high-entropy-gsqrco-nothink,
  extreme-precision-baseline}.json`, ladder RESULTs in this session