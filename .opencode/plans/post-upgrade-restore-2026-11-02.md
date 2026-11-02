# Post-upgrade daily-driver restore — 2026-11-02 19:39 CET

## Trigger

PC hardware upgrade, VITRIOL temporarily torn down. Journal shows the teardown
was exactly:

```
2026-10-30 22:00:49 sudo systemctl disable --now \
  vitriol-server.service vitriol-autosave.service vitriol-governor.service
```

No files, config, binaries, or `/scratch/kvtier` state were removed.

## Observed hardware (post-upgrade)

| item | value |
|---|---|
| GPU 0 | NVIDIA GeForce RTX 3060, 12288 MiB, cc 8.6 |
| GPU 1 | NVIDIA GeForce RTX 3060, 12288 MiB, cc 8.6 |
| Driver | 615.71.09 (KMD 615.71.09) |
| CUDA UMD | 13.4 |
| System toolkit | `/opt/cuda` = CUDA 13.4 |
| RAM | 39 GiB |
| Swap | 103 GiB total (zram 39 G prio 100 + 64 G partition prio 10) |
| `/scratch` | 375 G, 87 G free |

Notable delta vs build era: engine binary was built with CUDA 12.9
(`/home/randozart/toolkits/cuda-12.9`, arch `sm_86`). Its RUNPATH still
resolves the CUDA 12 runtime libs:

```
RUNPATH [/home/randozart/Desktop/Projects/VITRIOL/engine/build/bin:
         /home/randozart/toolkits/cuda-12.9/targets/x86_64-linux/lib:]
```

Verified with a minimal environment (`env -i`): `vitriol-server --version`
works and `ldd libggml-cuda.so.0` resolves `libcudart.so.12`,
`libcublas.so.12`, `libcublasLt.so.12` from the 12.9 toolkit. **No rebuild
required under the new driver.** Existing build kept (strategy: try first).

## Restore actions

Owner ran (no `enable`, by request — no autostart at boot):

```
sudo systemctl start vitriol-server.service   # pulls vitriol-autosave via Wants=
sudo systemctl start vitriol-governor.service
```

Preflight (read-only, before start): `scripts/vitriol serve --dry-run`
passed the sparse-KV guard and produced a fingerprint identical to
`~/.vitriol/fingerprint.last`. No stale engine processes.

## Result

| check | value |
|---|---|
| model load | 34.8 s to `model loaded` |
| `/health` | `{"status":"ok"}` |
| alias | `Lapis Occultus` |
| model | `Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf`, n_ctx 262144 |
| slot restore | `slot0.bin`: **39,312 tokens in 614.9 ms** |
| disk tier | restored 12 prompt states (10 skipped, 11 rejected, 0 unverified) |
| server unit | active/running, `Result=success`, `NRestarts=0` |
| autosave unit | active/running, `Result=success`, `NRestarts=0` |
| governor unit | active/running, `Result=success`, `NRestarts=0`, phase=Full |
| OOM/abort | none (`grep -icE 'oom|killed|abort|assert'` = 0) |
| swap in use | 0 B |
| VRAM after bench | GPU0 10,955 / GPU1 10,982 MiB (≥ 981 MiB free each) |
| host RAM | 20 GiB used, 18 GiB available |

### Fingerprint (unchanged)

```
VITRIOL-FINGERPRINT model=Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf
ts=27,21 c=262144 kv=q4_0/q4_0 fa=on ub=128 mode=off engine=vitriol-dma
ckpt=8192 ckpts=4 lru=0 sparse=sparse score=probe pool_reset=0 spec=mtp:3 par=1
ssp=1 cram=12288 cxr=1 ct=chat-template-tolerant.j2 cis=0
```

Matches the last pre-teardown launch exactly. The `fingerprint.blessed` file is
stale (2026-09-17, Q3_K_M / ts 24,24 / mtp:1) — expected, not re-blessed.

### Bench (`scripts/bench-shallow.sh post-upgrade 3`)

```
median_tps=28.993 runs=29.084 31.96 28.993
prompt_tps ≈ 25.17–25.45
cache_n=2228, predicted_n=64
```

Deterministic quality check (greedy, fixed seed, 3×): identical output every
run (`\n\nVITRIOL-OK`, 22.3–23.1 t/s). The bench SHA differs across long-prompt
runs (speculative batching), but greedy short-context output is stable.

`/metrics` after smoke: `prompt_tokens_cached_total 6684` (counter working),
`kv_ejected_total 0` (expected; cache fill below depth gate).

## State left behind

- All three units **active** but **disabled** — they will NOT autostart on boot.
- Governor phase Full; `/var/lib/vitriol-governor/state.json` unchanged.
- `/scratch/kvtier` mirror intact; `slot0.bin` restored.

## Fallbacks (not needed)

- OOM/VRAM at 262K → `vitriol config load qwen38-buffedmod-208k`, restart.
- MTP n=3 pressure → `draft_n_max=2`/`1`, restart, record fingerprint delta.
- Governor churn → `sudo systemctl stop vitriol-governor.service`.
- If a future driver/toolkit change breaks CUDA 12 runtime resolution:
  rebuild with `scripts/build-llama-server.sh --backend cuda`
  (`CMAKE_CUDA_ARCHITECTURES=86`).

## Follow-ups (optional)

- Re-bless `~/.vitriol/fingerprint.blessed` to the current point.
- Refresh stale AGENTS.md hardware lines (32 GiB / 16 G swap / 1070 Ti-era
  notes) to match 39 GiB RAM and the 64 G swap partition.
- The engine log used by `bench-shallow.sh` for the fingerprint line is
  `/tmp/opencode/vitriol_gen.log`, but the launcher now emits the fingerprint
  to the journal only; the bench prints "(no fingerprint line in engine log)".
