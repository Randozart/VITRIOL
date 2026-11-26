---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T10:49:21+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "d49aa713d5323235102c9273cb3b58d6f0fcc8ce"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-101857-kintsugi-replay-z440.md", ".opencode/plans/inkling2-per-head-fa-mask-2026-11-24.md"]
---

# Z440 Kintsugi and Inkling experiments

## Purpose

Improve terminal latency on Kintsugi, test the second GPU, and establish a working Inkling 2 Mini configuration.

## Approach and alternatives

Run matched Kintsugi draft lengths and independent GPU workloads. Test full draft SWA checkpoint retention in a private relink. Diagnose separate draft placement with an ordinary output buffer. Test locally available Inkling weights at a previously fitting window.

- Evaluate one-card Kintsugi, split target and drafter, an independent GPU0 endpoint, and resident Inkling. Favor reversible local tests before model downloads, library installations or hardware changes.

## Scope and documentation

Record experiments in this plan and the repository anchor. Run the temporary test binaries, GGUF and harnesses from `/tmp/z440-kintsugi-inkling`. The working launcher and service are used as baselines.

## Verification

- Record full argv, request timings, accepted drafts, CUDA telemetry, and output sanity. Require fresh and post-checkpoint coverage, both GPUs working where requested, and explicit tested context depth. Run scoped repository gates on delivered changes.

## Results and evidence

The report below links measured outcomes to the recorded fingerprints. Logs, response JSON, GPU samples, build inputs, and verification manifests are under `/tmp/z440-kintsugi-inkling`. Only the plan and repository anchor are intended for commit.

## Change history

- 2026-11-26T10:49:21+01:00 — Plan created before implementation.

Related: [Kintsugi launch baseline](2026-11-26-101857-kintsugi-replay-z440.md) and [Inkling depth investigation](inkling2-per-head-fa-mask-2026-11-24.md).

## Invariants

1. The default Kintsugi route still serves ordinary and exact-append requests. Check against the seven-token control. A restore experiment must preserve a contiguous prefix and activate drafting through the existing coverage check.
2. Any speed claim needs matched workloads, filled depths, and measured successful CUDA decode. A load failure is evidence about that shape.
3. System service, installed profiles, and existing build stay outside private experiments. Check readiness, cleanup, and profile digests.
4. For Inkling, use a coherent chat response and measured prompt fill. A configured window is separate from proven fill. Start with local IQ4_NL, resident layer split and ubatch 32. Official refreshed GGUFs are an alternative if the local format or engine compatibility fails.

Ideas to test: full draft-cache serialization; fixed draft lengths 3/5/7 and adaptive length; GPU0 drafter with ordinary shared output as a diagnostic for a future local projection; independent smaller model with Kintsugi still on GPU1; Inkling resident across both GPUs. Capture per-arm timings and host/GPU power samples in `/tmp/z440-kintsugi-inkling`. If full retention fails coverage or creates recurring memory/latency regressions, hold production changes. The second GPU only wins for single-stream use if it improves matched coding latency after shipping activation costs and any output-head changes.

## 2026-11-26T10:52:00+01:00 Inkling regression found

The original 32K/ub32 control requested 15861 MiB on CUDA0 at model load. Commit 3394a8d made the placement denominator count all logical layers again, although Inkling stores weights only for the physical layers. Test a private copy using the physical layer count to apportion the weights and keep the logical range for output offload. Retain the alternative of a reduced CPU MoE placement if resident split still fails. Validate Kintsugi on the same source before promoting a loader change.

## 2026-11-26T10:57:00+01:00 Kintsugi draft length and placement

Four synthetic coding tasks plus one repetition control, 192 output tokens per request. Same Kintsugi-27B-12G target file and DFlash2 drafter, CUDA engine, 65536-token window, full draft retention, and GPU-resident recurrent snapshots. Coding entries summarize the four distinct tasks. GPU energy estimates multiply sampled board power by decode duration and divide by generated tokens. The integration uses the JSON response file timestamp as the approximate decode end, so the figures exclude CPU and PSU draw and have sampling error.

| Arm | Coding median tok/s | Approximate median GPU J/token | Repeat tok/s |
| --- | ---: | ---: | ---: |
| Fixed 7, GPU1 | 39.45 | 3.29 | 67.98 |
| Fixed 3, GPU1 | 32.39 | 4.03 | 41.81 |
| Fixed 5, GPU1 | 35.82 | 3.63 | 56.92 |
| Adaptive up to 7, GPU1 | 36.97 | 3.51 | 65.87 |
| Target GPU1, drafter GPU0 with local exact Q8_0 projection | 32.56 | 4.33 | 58.50 |
| Target GPU1, drafter GPU0 with local projection and HEAD2 on both | 32.91 | 4.22 | 54.10 |

Keep fixed 7 for this corpus. The original separate-device arm failed with the borrowed projection even when HEAD2 was disabled and the target projection was in an ordinary CUDA buffer. A private draft GGUF with a byte-identical copy of the target projection, [5120, 248320], 1350860800 bytes, SHA-256 `06dbe1f91f939d44cce18e62d71d6ed5555eb46142682e33d4dc22bf6b820030`, and the private loader routing that projection to GPU0 resolved startup. The five response texts and acceptance counts matched fixed 7. The instrumented lattice/readout interval increased from 15.72 to 37.94 ms per call at 200 samples. That interval includes GPU waits and measures the entire readout phase. See `draft-local-projection.log` and the matched `draft-7.log`. The independent HEAD2 registration variant served the same matched request set on the existing build with the copied GGUF. It remained slower and used more sampled board energy per generated token than the one-card control.

## 2026-11-26T10:57:00+01:00 full restore prototype

The existing engine serialization skips attention-masked SWA rows during sequence save. Full-size draft allocation still masks old rows at attention time, so a restore loses older draft positions. The isolated relink retained those rows when opted into for this process. The existing DFlash2 coverage and target verification paths ran as before.

| Arm | Fresh 4087-token prompt decode | Restored suffix decode | Restored target tokens | Checkpoint |
| --- | ---: | ---: | ---: | ---: |
| Existing engine | 45.73 tok/s | 12.64 tok/s | 4083 | 160.923 MiB |
| Full retention prototype | 42.84 tok/s | 47.03 tok/s | 4083 | 172.148 MiB |

The follow-up added 11 input tokens and generated 48. The prototype accepted 39/53 draft tokens after checkpoint restore. Both response texts matched the control. The process-wide diagnostic switch is for the private binary. A production change should bind retention to the explicit full-SWA draft-cache setting and budget checkpoint growth. Keep the normal compact format and verify that old trimmed states still fall back safely.

Source ledger: the model author lists llama.cpp as a supported inference route. The ggml-org IQ4_NL release uses 17.6 GB, with 24 physical layers and 48 evaluations per token. The local file and private CUDA runs are the evidence for this host. Public alternative engines and quants were screened from their primary documentation; they were not downloaded or timed. [Model author card](https://huggingface.co/thinkingmachines/Inkling-2-Mini) · [GGUF release](https://huggingface.co/ggml-org/Inkling-2-Mini-GGUF) · [Upstream architecture port](https://github.com/ggml-org/llama.cpp/pull/32694).

## 2026-11-26T10:58:11+01:00 Inkling on the existing build

The private equal-split correction loaded both cards. On the existing binary a `1,3` logical split reproduced the exact physical allocation because 13 of the 24 physical layers fall below both the corrected half-way threshold and the existing quarter-way threshold. It loaded 7543 MiB of model buffers on CUDA0 and 8788 MiB on CUDA1. Peak sampled use was 10676 and 10608 MiB respectively. This is a workaround for this local Inkling GGUF and current binary. A future physical-denominator fix changes the interpretation of the ratio.

Text chat worked with both cards, no drafter, a 32768-token configured window, 32-token ubatch, and F16 KV. The 28-token Paris prompt decoded at 20.57 tok/s. A 2335-token filled recall prompt returned ORANGE at 18.10 tok/s. Both visible answers included the model's literal `<|end_message|>` suffix. A coding request generated 256 tokens at 25.46 tok/s, used its whole budget in the reasoning channel, and returned empty final content. Vision, tool calls, the full configured window, and DSpark remain outside this validation.

Run the verified configuration while the other engine is stopped. This takes both cards and serves the existing local model on port 18301.

```bash
(
  set -e
  bash scripts/engine-launch-gate.sh pre
  export CUDA_DEVICE_ORDER=PCI_BUS_ID CUDA_VISIBLE_DEVICES=0,1
  export VITRIOL_MODE=off LLAMA_HEAD2=0 GGML_CUDA_SHARED_POOL=1
  export VITRIOL_RS_VRAM_FRACTION=1 VITRIOL_KV_VRAM_CELLS=0
  export LLAMA_RS_REPLAY=0 VITRIOL_RS_REPLAY_OP=0
  argv=(engine/build/bin/vitriol-server
    -m /scratch/models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf
    --alias "Lapis Occultus" --host 127.0.0.1 --port 18301 --fit off
    -c 32768 -b 512 -ub 32 -ngl 99 --device CUDA0,CUDA1 --split-mode layer
    -ts 1,3 -np 1 -t 4 -fa on -ctk f16 -ctv f16
    --ctx-checkpoints 4 --cache-ram 256 --reasoning off -lv 4)
  printf 'VITRIOL-FINGERPRINT: gpu=0,1 mode=off head2=0 pool=1 rs=1 kvc=0 replay=0 argv='
  printf '%q ' "${argv[@]}"
  printf '\n'
  exec "${argv[@]}"
)
```

## 2026-11-26T10:58:11+01:00 independent second-GPU work

With Kintsugi fixed to GPU1, a separate Qwen2.5-0.5B Q4_K_M server on GPU0 completed a simultaneous 192-token request. Kintsugi decoded its same 192-token coding prompt at 41.28 tok/s alone and 41.69 alongside the small model. In this one pair, elapsed time for both responses was 5.36 s with overlap and 6.18 s in series. Treat the result as a feasibility check for independent requests; the models serve different capability tiers.

- 2026-11-26T11:00:32+01:00 — Stock Inkling 1:3 split passed chat and 2335 filled-token recall. Fixed seven wins the tested draft sweep. Full-SWA checkpoint retention recovered drafting at 4K; the 33K validation and extra follow-up arms are in progress.

## 2026-11-26T11:01:13+01:00 deep restore validation

The previous 33116-token workload also passed with the private relink. Fresh 64-token decode ran at 41.42 tok/s with 55/55 drafts accepted. The suffix replacement restored a 332.274 MiB checkpoint containing 33112 input positions, evaluated 11 new tokens, accepted 25/39 drafts and generated 32 tokens at 31.28 tok/s. The earlier checked-out launcher ran that follow-up at 8.99 tok/s without drafting. Time reported for prompt plus decode was 1.78 s in the prototype and 4.02 s in the earlier control. Response text matched on both arms. Fresh decode in the earlier control was 46.11 tok/s, so the measured benefit is concentrated on restoring a diverged conversation. The deep control is from the previous session under the same workload and launch options. The closing check also confirms the prototype's 4K result with a same-session control.

Material implementation tradeoff: full draft snapshots consume more host memory and take longer to save and restore. The production candidate should honor the existing request for full draft retention at cache construction, preserve compact snapshots by default, document the host-memory budget, and exercise compatibility with old compact snapshots. The private relink and copied GGUF stay under `/tmp/z440-kintsugi-inkling`. The baseline and three changed C++ translation units were compiled or retained from the checkout as described by the private relink. Praetor reported zero diagnostics in those isolated source comparisons.

## 2026-11-26T11:02:40+01:00 Inkling reasoning control

For this GGUF, pass `"reasoning_effort":"minimal"` in the chat request. The template maps it to 0.1. This engine's request handler erases the template effort override for the string `none`, and the template then uses 0.9. The same 256-token coding request with `minimal` returned the complete merge function in the final answer at 25.32 tok/s. The explanatory text reached the token limit. Paris and the 2335-token ORANGE recall still returned correctly, with the same literal message suffix.

All launches were ordinary user processes on temporary loopback ports. No sudo, driver patch, new model download, or system-service activation was needed. The original VITRIOL config and measured Kintsugi profile digests matched afterward.

- 2026-11-26T11:03:02+01:00 — Draft sweep, checkpoint recovery at 4K and 33K, both replicated-head layouts, independent-GPU overlap, stock Inkling launch, and minimal reasoning were measured. Only the experiment record and repository anchor are delivered to the main tree.
