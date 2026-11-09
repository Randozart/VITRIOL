# Daily driver performance — GPUs unlocked

- Date: 2026-11-09 17:22
- Host: dual RTX 3060 12GB (both now visible)
- Service: `vitriol-server.service` SYSTEM scope, active
- Model: `Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf`
- Config: daily driver `~/.vitriol/config`

## Fingerprint

```
VITRIOL-FINGERPRINT model=Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf
ts=27,21 c=262144 kv=q4_0/q4_0 fa=on ub=128 mode=off engine=vitriol-dma
ckpt=8192 ckpts=4 lru=0 sparse=sparse score=probe pool_reset=0
spec=mtp:3 par=1 ssp=1 cram=12288 cxr=1 ct=chat-template-tolerant.j2 cis=0
```

Load: healthy after ~80 s. No OOM/errors in the service log.

## Shallow bench

`scripts/bench-shallow.sh dual3060-unlocked 3`:

| run | decode t/s | prompt t/s (cached) |
|---|---:|---:|
| 1 | 29.07 | 24.99 |
| 2 | 28.86 | 24.28 |
| 3 | 30.38 | 24.92 |

Median: **29.07 t/s** decode on the 1.2k fixed prompt.

Prior certified live unit bench (2026-09-03, ts 22,14): 16.49 t/s.
Current config is roughly **+76%** shallow decode.

## Depth (filled context)

Realistic prompt assembled from `AGENTS.md`, `docs/OFFICINA.md`,
`docs/cpp26-conventions.md`, and `scripts/vitriol`; `cache_prompt=false`,
64 greedy tokens.

| filled prompt | prefill t/s | decode t/s | MTP accept |
|---:|---:|---:|---:|
| 19,807 | 433.58 | 22.45 | 52.1% |
| 23,623 | 426.42 | 20.08 | 43.2% |
| 35,691 | 404.28 | 18.69 | 42.2% |
| 70,005 | 366.59 | 11.86 | 20.9% |

Prior depth certification (2026-09-04, ts 22,14, MTP on):
12.66 / 11.16 / 10.45 t/s at 13K / 26K / 36K.

Current rig:
- ~**+77%** at 20K
- ~**+80%** at 24K
- ~**+79%** at 36K
- 70K filled now completes with no OOM (old reference wall was 54,692
  tokens on the 1070 Ti-era tq3_0 config; not apples-to-apples, but the
  previous dual-3060 profile did not certify this depth).

## MTP

Cumulative after the runs: acceptance **57.2%**, **1.69 accepted tokens per
verification step**. Acceptance decays with depth (52% -> 42% -> 21% at 70K),
which matches the decode-rate falloff.

## VRAM / stability

- GPU0: 11,026 MiB used / 930 MiB free
- GPU1: 10,984 MiB used / 979 MiB free
- Temperatures after 70K run: 80 C / 74 C
- `kv_ejected_total`: 0
- Health stayed `ok`; no OOM, abort, or watchdog events.

## Verdict

Daily driver loads cleanly on both 3060s and is materially faster than the
last certified profile. Shallow: **~29 t/s**. Working depth: **~20-22 t/s
around 20-24K**, **~18.7 t/s at 36K**, **~11.9 t/s at 70K**. The old
~54K wall did not reproduce at 70K.

Service was left running.
