# 2026-11-19 09:26 — PTL CPU buffers, repacking and current voice latency

Research documentation only in VITRIOL. Avatar owns the tested private telemetry,
launchers and evidence. No engine code, default profile, home-PC runtime or voice
weights changed. Current native PTL build1/8e16421/GNU16.2.1, current20B MXFP4,
Nano NPU/CPU4, B1/source ears/720×1280/25FPS retained.

## Invariants

1. I1: restore original live version, model, voice, command and device pins;
   release admission, binary/launcher hashes, exact argv and CPU masks pin this.
   Generator PID/cache warmth intentionally changes after authorized stop.
2. I2: allocated DRM bytes, backing, ordinary RSS, file cache and memcg charges
   remain separate views; never add overlapping quantities. Four Avatar tests
   pin device/client deduplication, raw fields, maps and unavailable observations.
3. I3: same20B/currentvoice/face and acceptance thresholds in every playback arm;
   all5STT replies/WAVs exact, all5server/browser gates pass. Transitional control
   and duplicate-generator reference are excluded from matched latency claims.
4. I4: private CPU residency requires actual anonymous RSS covering model bytes,
   zero VmSwap and private SwapMax0, not flags alone; snapshots before/after warmup
   and requests pin admission. No claim of continuous page-level locking.
5. I5: isolated token/synthesis improvements do not imply total latency gains;
   actual browser clocks and original voice waveform parity remain required.

## Exact configuration and allocation

PTL Core Ultra X7 358H/Arc B390/AI Boost,16logicalCPUs/about62GiBRAM. Same original
GPT-OSS20B file12,109,566,624bytes, previously qualified SHA256
27cd6c432c7672cb812a92f611cf3ba7bbc35928262bb1e1253ff4ee6ae35901; no model writes.
Full argv/fingerprint for each private native launch is in Avatar evidence.
All ngl0/t12/np1/c8192/FAauto/reasoning-budget0/cache-reuse256/cacheRAM256MiB/
checkpoints4/no-idle-slot-backup/lazyoff. Private generator quota1200%,
MemoryHigh14GiB/Max16GiB/SwapMax0; app1200%/20GiB/24GiB. Current Nano17graphs/
CPU4 and whole-model WhisperNPU unchanged. Window8192 is not filled-depth proof:
177sharedprefix/~9–13new prompt tokens plus actual generated replies.
Native says cache_reuse unsupported and disables it; real cache_n177 is retained.

Explicit mmap control uses CPU_Mapped11536.18MiB. None/no-host-off uses
Vulkan_Host11536.60MiB with small ordinary anonymous RSS. Loader source can disable
AUTO mmap on integrated GPUs and prioritizes GPU-host buffers before CPU-extra.
Private DRM snapshots resolve the earlier post-run-only attribution gap.
Requested and resident categories are distinct; same device/client IDs deduplicate
descriptors. Neither zero resident-system nor small RSS proves all weights absent.
Reference: https://docs.kernel.org/gpu/drm-usage-stats.html.

None/--no-host selects CPU1854.15MiB+CPU_REPACK9682.03MiB, still uses Vulkan compute
99.50MiB/Vulkan_Host65.04MiB. Anonymous RSS12,432,805,888bytes after model warmup,
12,438,953,984after first turn; repeat agrees within pages. VmSwap0, private memcg
~12.45–12.55GB. CPU-extra/repacking means this is not solely a residency change.
No global swap changes, full-model mlock or data deletion.

## Full playback results and limitations

Serial mmap reference → transitional host → CPU → CPUrepeat → reversehost.
Original generator intentionally stopped during transitional first turn to remove
the second12GBmodel. Its timing is excluded; mmap reference has duplicate condition.
Stable single-generator observations:

| Arm | First content | Server first audio | Server first video | Browser main audio |
|---|---:|---:|---:|---:|
| CPU/repack |.734s|4.477s|6.741s|6.829s|
| CPU/repack repeat |.695s|4.745s|6.753s|6.863s|
| Reverse Vulkan-host |.843s|4.877s|5.812s|6.122s|

All5STT texts/WAVs exact. All5server/browser gates pass:311expected browser slots,
0missing/duplicate/skipped/refill/overlap/longpause,40–70msmaxdrift, captions/handoff
pass. No gate tolerance change or physical-speaker judgement. Fixed order, two CPU
observations/one stable host; no population percentile/general speedup claim.
CPU storage qualifies, but later video/browser means NO live promotion.

Reverse-host73,579majorfaults/147,456read_bytes vs CPU9/8faults/49,152read_bytes.
Do not translate faults into SSDbytes or infer per-stall kernel causes from this.
https://docs.kernel.org/filesystems/proc.html defines read_bytes separately.

## Native ablation and remaining voice costs

Separate native-only same20B study,3questions×2rounds in5serial arms,177prefix prime.
All30complete texts match host. Host/reversehost decoding medians21.47/20.72t/s;
CPUrepack/reverseCPU23.86/23.64; CPU no-repack20.95. First-content medians
.629/.676, .580/.577, .669s respectively. Benefit follows repacking; not a pure
physical-residency attribution and not a whole-avatar speedup.

Current NpuVoice component profile,2phrases×2trials, original4CPUthreads/estimator
graphs/reference/seed41–42. All4float32outputs exact versus uninstrumented controls.
6.16s phrase: total2.313s, T3tokens1.309s, HiFT.583s, flow.406s (encoder.208,
estimator.196 inside flow), watermark.014. 3.28s phrase total1.344/tokens.733/
HiFT.302/flow.300(estimator.146). Nested rows must not be summed again.
Host wall time, no NPU device-timestamp or production-tail claim.

Next: split T3 prefill/decode before selecting acceleration or a lighter student;
test T3/HiFT placement against concurrent motion/STT/estimator and full browser
latency. Already-fast estimator offers smaller opportunity. Distillation is a
quality-gated fallback, no new training launched or current voice replaced.

## Guards, restoration and research ownership

Initial09:06–09:07warmup-pressure guard abort retained (17.8GBavailable/PSIfull14.23).
Bounded transient warmup allowance revised, playback thresholds unchanged.
Retry09:08–09:09server control passes; missing temporary browser parent aborts
before browser launch, retained. Corrected full09:10:10–09:18:11 and component
09:18:50–09:20:54 complete, independent25/9minrestore timers and exit traps.
Restored original live unit/command/model/voice/pins, new generator PID2230472,
all thread masks0–15, Performance retained, extraction thawed, no private units/
timers active. Publicv9 ready/clientACKready09:22. Older PID-bound mask helper
reports expected missing original PID after intentional restart; explicit new-PID
argv/masks verification closes that limitation. Warm KV/cache state not preserved.

Avatar stage62abfec adds read-only allocation telemetry.278runtime tests,
backend743pass/5skip/Rust94/type/lint/format pass. Source snapshots, argv/graphs,
raw counters, failed gates, native/voice CPU/memory journal accounting, analyzer,
scientific plot and restoration proof in Avatar assets/ditto-live/cpu-buffers-20261119.
Main report: docs/research/2026-11-19-cpu-buffers.md in Avatar. Root SSD nearly full;
large videos/WAVs retained only at hashed temporary paths, not durable media backup.
Compact evidence committed. No deletion of models/recordings/source/recovery data.
Intel/WBSO findings documented;0automaticemployeeS&Ominutes, human review pending.
No Intel message sent, no home-PC/profile/runtime changes.
