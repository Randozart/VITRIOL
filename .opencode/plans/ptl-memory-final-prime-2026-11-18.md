# 2026-11-18 16:30 — PTL final priming and memory attribution

No VITRIOL runtime or production profile change. Current native engine8e16421,
GPT-OSS20B MXFP4 SHA27cd6c432c7672cb812a92f611cf3ba7bbc35928262bb1e1253ff4ee6ae35901,
current Nano/B1 and publicv9 retained. Research/implementation is in Avatar.

## Invariants

1. I1: public model/voice/face/config remain unchanged; exact restoration and
   original20B PID1304035/command/pins checked after the approved guard.
2. I2: final priming runs only after renderer/STT/TTS warmups, ownership stays
   stable and failed priming blocks readiness; seven lifecycle tests pin order,
   retries, healthy idle, cancellation and worker replacement.
3. I3: no residency claim from flags or zero swap alone; two tests consume actual
   process/policy/size facts, and the unverified variant is refused.
4. I4: numerical/content and A/V gates precede timing; actual first speech text/WAV
   exact, server gates and both browser gates pass, no tolerance changes.

## Experiment and exact observations

Same private CPU-layer config:ngl0/t12/one slot/context8192/FAauto/cache-reuse256/
cacheRAM256MiB/checkpoints4/no-idle-backup; explicit load-mode mmap/lazy-mode off.
Both mapped arms MemoryHigh16GiB/Max18GiB, quota1200%; app20/24GiB/quota1200%.
These are NOT the public automatic loading defaults; no cross-study causal claim.
Actual argv/fingerprints in Avatar memory-final-prime-retry-20261118/*-llm.log.

One real STT first turn and one later typed browser turn per valid arm, fixed
order early→late→attempted resident. Early first content1.5487s/audio5.3375s/frame
8.2097s; late .7659s/4.4764s/5.6460s. Reply text/WAV exactly equal. In-request
LLM major faults14441→1233, read_bytes1302044672→79167488. TTS4.518→4.631s,
so the improvement is in total response, not every component. Browser7.7634→6.0290s,
both40msmaxdrift/0skips/holes/duplicates/longpauses/overlap/refill;311replyslots.
Single paired observation, no production percentile or universal speedup claim.

Mapped-file cache:early12.11GB before warmup→10.4GB after; late12.11→4.2GB after
warmup→11.3GB after final prime. Warmup moves about333–336MB of engine anonymous
state to swap; final prime touches the hot subset. First-turn disk faults/reads
support moving work out of the user turn; do not infer all latency causality.

The none/off loading + private MemorySwapMax0 attempt has0VmSwap but only~.338GB
RssAnon versus12.11GB model file. Anonymous-buffer admission rejects it and no
conversation/playback is run. This does NOT prove physical non-residency, a load
flag no-op or insufficient capacity. Loader source8e16421 can select GPU-host
buffer types for CPU layers; auto mmap is disabled on devices reporting lack of
mmap support. CPU-layer configuration does not fully describe buffer placement.

Post-run original PID1304035 fdinfo:xe client792 drm-total-system12116036KiB,
drm-resident-system0, drm-resident-gtt21600KiB. This is a post-run production
observation, not private-variant telemetry. Kernel DRM docs distinguish requested
allocation from instantiated resident backing; neither total nor RSS alone is
sufficient proof. Do not sum duplicate device/client entries. More backend/DRM
attribution, or an explicitly CPU-buffered --no-host experiment, is needed.
Reference:https://docs.kernel.org/gpu/drm-usage-stats.html.

## Bounds, negatives and deployment

Initial harness failed before private launch because readiness was checked too
soon; exact restoration worked. Corrected retry stayed within SAME original
16:04:55–16:14:55 approval, shortened to1STT+browser per arm. Independent timer
restored at16:13:29; public warm/ACKready observed by16:14:24. Residency attempt
stayed unready and was stopped at the deadline, no relaxed proof or extra window.
Private swapmax never applied globally; no mlock, file deletion or home-PC changes.
Performance and original20B/currentvoice/B1 preserved. Late placement selected as
private default; early remains explicit control.273runtime/backend743+5skip/Rust94/
type/lint/format pass; nine new tests. Model window capacity8192 is not filled-depth
certification; observed prompts177–190tokens plus generated replies.

Full research/plots/raw/source hashes/recordings are in Avatar docs/research/
2026-11-18-memory-final-prime.md and assets/ditto-live/memory-final-prime*. No live
promotion, no deep-context/home-GPU certification.0automaticemployeeS&Ominutes,
human review pending; no Intel message sent.
