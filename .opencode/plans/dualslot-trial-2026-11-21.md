# Dual-slot trial plan (2026-11-21)

Research: feature-exchange-and-dualslot-research-2026-11-21.md (same day).

## The decisive unknown — ANSWERED from source

`engine/src/llama-context.cpp:300`: with `kv_unified` (which the launcher
ALWAYS passes — `scripts/vitriol:2759`), `n_ctx_seq = n_ctx`: **each slot gets
the full 262144-token window**, sharing one KV buffer. Without it the window
would divide (131072 per slot). The unified buffer is sized once (4.78 GiB KV
total) regardless of slot count.

So the trial's decisive unknown is resolved WITHOUT a restart:
- per-slot window stays **262144** ✔
- VRAM: KV unchanged; one extra slot's compute buffers (~0.5-1 GiB) ✔
- DRAM: 39 GiB host, 10 GiB available — the 8-25 OOM class (16 GiB box) is gone ✔

## Trial config

Derive `lapis-dualslot-2026` from the live config with ONE change:
`parallel = 2`. Everything else byte-identical (ts 27,21, kv q4_0/q4_0,
c 262144, MTP n2, cram 16384, cache_max_gb 40, kvtier path).

## Steps

1. `cp ~/.vitriol/config ~/.vitriol/config.pre-dualslot` (rollback anchor).
2. Create `~/.vitriol/profiles/lapis-dualslot-2026/` = live config +
   `parallel = 2`; `vitriol config load lapis-dualslot-2026`.
3. Fingerprint diff (mandatory): old vs new `LAUNCH` lines in
   `~/.vitriol/fingerprint.log` — expect exactly `par=2` vs `par=1` delta.
4. Unit restart (`systemctl restart vitriol-server`); wait for /health.
5. Measure:
   - `/slots`: n_slots == 2, per-slot n_ctx == 262144.
   - VRAM delta (nvidia-smi before/after).
   - Shallow bench 3×64 single-slot t/s vs today's baseline.
   - MTP acceptance unchanged?
   - `/metrics`: kv tokens capacity.
6. Smoke: one Officina turn through the local lane (background-review gate
   auto-activates at ≥2 slots — verify it fires).
7. Report in `.opencode/plans/`; rollback = `vitriol config load` the saved
   pre-dualslot config + restart.

## Rollback

`cp ~/.vitriol/config.pre-dualslot ~/.vitriol/config` + unit restart.
The single-slot daily-driver profile is untouched.

## Note on the request path

`vitriol-server` serves both slots from ONE process on 8279 — no Officina
change needed. The background-review gate activates by slot count alone.

## RESULTS (executed 2026-11-21, 11:54–12:00)

| Measurement | Value | Verdict |
|---|---|---|
| Slots | **2 × 262144 tokens each** | decisive unknown resolved — full window per slot |
| VRAM delta | +264 / +172 MiB (10.8/11.1 GiB of 12+12) | ~3 GiB headroom remains |
| Fingerprint | `par=2`, all else identical to baseline | only the intended delta |
| Slot0 solo predict | 17.3 tok/s (64 tok), 23.5 tok/s (256 tok deep) | healthy |
| Slot1 solo predict | 18.99 tok/s | symmetric |
| Concurrent 0+1 (128 tok each) | 15.54 / 17.94 tok/s | ~85% aggregate — good scaling |
| kvtier | 59.2 GiB, 12 turn checkpoints, under the 40 GiB trim? — trim is on cache+slot files; turn files are the extension's ring (keep-3) | watch next session |
| Doctor | routing proxy in path, gateway ok | post-restart supervision fine |
| MTP | n2 active (fingerprint), no acceptance regression observable in 256-tok run | needs longer soak for acceptance stats |

Verdict: trial SUCCEEDS. Both slots serve the full 262144 window; concurrent
throughput ≈85% of solo (excellent for shared compute); VRAM headroom ~3 GiB;
Officina's background-review gate auto-activates (2 slots detected).

Rollback preserved: ~/.vitriol/config.pre-dualslot (single-slot config) +
`vitriol config load` the saved daily-driver profile + unit restart.
