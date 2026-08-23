# REBIS memory model — where every gigabyte goes

The 15 GB host RAM + 12/8 GiB VRAM split is the binding constraint. This is
the full accounting, with the incidents that taught each line.

## Host RAM (15 GB)

| consumer | steady | notes |
|---|---|---|
| Sol prompt cache | ≤2048 MiB (`--cache-ram`) | bounded after OOM incident; default 8192 |
| Luna prompt cache | ≤1024 MiB | same class of incident |
| VITRIOL checkpoints | 12/slot × ~150 MB at big ctx | `--ctx-checkpoints` bound; spacing via `-cpent` |
| mmap'd weights | page-cache, evictable | NOT anon-RAM; survives pressure |
| pinned DMA buffers (VITRIOL) | GB-scale when streaming | only when a head streams experts |

Incident A: default `--cache-ram 8192` + long session → anon-rss 7 GB →
OOM kill. Incident B: dual load with `--no-mmap` staged 6.2 GB through host
RAM simultaneously → collision. Fixes: bounded caches, mmap weights,
staggered starts.

## VRAM (12 + 8 GiB)

| head | weights | KV @64k q4_0 | buffers | total |
|---|---|---|---|---|
| Sol IQ2_S | ~8.4 GiB | ~0.9 GiB | ~1 GiB | 10.2/12 |
| Luna IQ4_XS | ~6.2 GiB | ~0.7 GiB (SWA!) | ~0.5 GiB | 6.87/8 |

Luna's sliding-window attention (3:1, window 1024) is why its long-context
KV stays tiny — the property that makes pinning at 64k possible at all.

## The reuse economics

Prefill (consumption) runs 400–560 tok/s across both heads; decode runs
20 (Sol) / 70 (Luna). Consumption is 6–28× cheaper per token than generation.
Every architecture decision in REBIS exploits this: route judgment to the
smart-slow head sparingly, draft on the fast head massively, and keep caches
warm so consumption approaches zero.

Measured anchor points: same-prefix re-request 46.95s → 0.06s (gated cache);
compaction event costs one cold re-prefill (~47s at threshold size) then
returns to warm operation.

PROVENANCE: measured on this rig; see EXPERIMENT_LOG.md and
docs/REBIS_FLAGS.md for flag-level semantics.
