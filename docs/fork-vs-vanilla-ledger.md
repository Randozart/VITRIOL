# Fork vs Vanilla Ledger — VITRIOL llama.cpp on the Daily Driver

**Date**: 2026-10-12. **Question**: does the VITRIOL fork earn its keep
over vanilla llama.cpp on this box (RTX 3060 + GTX 1070 Ti, 32 GB RAM,
Qwen3.8-27B BuffedMod IQ3_S_XL Q6K, deep-context single-user workload)?

**Verdict**: yes for this workload, with measured deltas — and real,
bounded costs. Everything below is measured; dormant features are listed
as dormant.

## Measured wins (live config: `qwen38-buffedmod-262k`)

| capability | delta vs vanilla | provenance |
|---|---|---|
| Internal MTP decode | +40% shallow (11.79 -> 16.46 t/s, controlled A/B); +33% at depth (16.1 vs 12.1 t/s @ 66.8k filled); 36.6 t/s in today's round-2 arm | `mtp-flag-drift-postmortem-2026-09-03.md`, `fastmtp-d2t-results-2026-10-11.md`, round-2 report |
| Context window per VRAM | 262144 native on 12 GiB duo via sparse probe-scored KV eviction; vanilla-style full residency dies at ~45-61k on this hardware | `lull-phase0-report-2026-08-24.md` |
| Warm restarts | 745/755 prompt-cache tokens restored, ~0.5 s vs minutes of re-prefill (~170 t/s prefill; 100k session = ~10 min) | `restart-day-report-2026-10-11.md` |
| Long-session survival | sparse eviction + preservation keeps deep sessions alive where vanilla OOMs or caps the window | `officina-context-lifecycle-2026-09-03.md` |
| OOM posture | engine was the system's OOM kingpin (5 kills/day pre-trim); now `oom_score_adj` -500, VmSwap trimmed, hardened swapfile | `oom-reservoir-trim-2026-09-04.md` |
| Foreign-tenant survival | cooperative-VRAM governor (RELIEVE/RESIZE/YIELD ladder), live 2026-10-11 | `restart-day-report-2026-10-11.md` |
| Provenance discipline | per-launch fingerprint; config keys are flags; drift is diffable | `mtp-flag-drift-postmortem-2026-09-03.md` |

Vanilla llama.cpp runs the base GGUF (standard qwen3.5 tensors) but gets
none of the above: no internal MTP on this arch, smaller safe window, cold
restarts, no eviction/tiering, no governor.

## Fork features DORMANT on the current driver (honest list)

| feature | status | why |
|---|---|---|
| Streaming/DMA engine (`VITRIOL_MODE`) | **off** | weights fit VRAM; streaming a fitting model is a pessimization (residency rule 2026-08-24). Insurance for 35B-class models |
| TurboQuant KV (`tq3_0`, -22% KV bits) | certified, not in live config | KV is not the binding constraint at q4_0 (~19.1 KiB/token); re-cert pending if depth wall ever binds |
| LoRA runtime (`--lora`, hybrid arch) | works, adapter shelved | only shared candidate measured -16% decode with zero green-rate gain (`lora-workstream-final-2026-10-12.md`) |
| eval-gate harness | ready, idle | `scripts/eval-gate/` — reusable for any future A/B |

## Fork tax (measured incidents)

- **Flag drift**: a config retarget silently dropped `[spec]`, costing 40%
  decode for weeks (2026-09-03). Remedy: fingerprint doctrine, now
  load-bearing.
- **Fork-introduced crash**: identical prompt resends (client retry after
  cold start) hit the fork's strict `GGML_ABORT` on recurrent rollback
  refusal -> 4x watchdog bounce loop (2026-10-12). Vanilla handles the
  same path by IGNORE the failure = silent KV corruption (invisible, and
  in our view worse). Fixed honestly: removal refusal falls back to full
  re-prefill; restored states try the checkpoint ring (inner
  `953784096`, `9b11ccb27`).
- **Maintenance**: permanent rebase/port burden vs upstream (inner `main`
  tracks ggml-org); every fork subsystem is code nobody upstream
  reviews.

## When vanilla would win

Short contexts, no restarts, no foreign tenants, upstream support cadence
valued over capability, and tolerance for smaller windows. None of those
describe this workload.

## Ledger rule

Any new fork capability enters this table only with a measured delta and
a provenance link. Dormant features are re-evaluated when the workload
changes, not before.
