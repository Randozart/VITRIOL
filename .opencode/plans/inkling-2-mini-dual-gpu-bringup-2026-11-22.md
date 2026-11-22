# Inkling-2 Mini on two GPUs — bring-up plan (2026-11-22)

Research: `.opencode/plans/inkling-2-split-across-two-gpus-research-2026-11-22.md`.
Owner confirmed the split bring-up and, if it passes, the VITRIOL profile.

## What is actually broken

The 27B daily driver splits across both cards with
`-ngl 99 -ts 27,21 --no-mmap` and **no** `--split-mode` flag (default
`LLAMA_SPLIT_MODE_LAYER`, `common/common.h:485`). The Inkling bring-up did not,
for two separate reasons:

1. **`--split-mode row` is the wrong mechanism** — it needs split buffers, which
   this CUDA backend rejects (`device CUDA0 does not support split buffers`).
   LAYER split (the 27B's mode) needs no split buffers. This was my mistake.
2. **A single 16.6 GB allocation landed on CUDA0** (~94% of the model) even when
   `-ts 12,12` was passed. The gdb backtrace from the aborted run shows the
   caller chain `common_init_result → common_fit_params →
   common_params_fit_impl → common_get_device_memory_data_impl → llama_context
   → graph_reserve`. **`llama-cli` runs the "fit" pass, which recomputes
   placement from free device memory** — it is the most likely reason the
   explicit `tensor_split` was not honoured. The `vitriol-server` path the 27B
   uses does not fit the same way. This stage must be settled from source before
   re-running anything.

## Stages

### S1 — settle the fit/tensor-split interaction from source (no model load)

Read, and record with file:line:

- `common/fit.cpp` — what `common_params_fit_impl` mutates (`tensor_split`?
  `n_gpu_layers`? `split_mode`?), and under what conditions it runs.
- `common/arg.cpp` — the flag(s) that disable or bound the fit pass (`--fit*`),
  and whether `llama-cli`/`llama-server` default it on.
- `tools/cli/main.cpp` + `common/common.cpp` — whether `llama-cli` can bypass fit
  entirely (e.g. a direct non-server path) so `-ts` is authoritative.

Deliverable: a short `## S1 findings` addendum appended to the research note with
the exact flag to use. **No GPU work until this is answered** — otherwise the
next run repeats the same misdiagnosis.

### S2 — split bring-up gate (I6)

With `VITRIOL_MODE` unset (disabled), default LAYER split, `--no-mmap`:

```
llama-cli -m ~/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf \
    -ngl 99 -ts 27,21 --no-mmap -c 16384 -fa off -n 16 --temp 0 -e \
    -p 'The capital of France is'          # + the S1 fit-disable flag
```

Gate (all must hold):
- **both** cards report non-trivial used memory during the run (`nvidia-smi`
  sampled while it loads), and **no** single allocation ≥ ~12 GB is attempted;
- 704/704 tensors load, 0 missing;
- coherent English continuation of the prompt (not token soup);
- exit status 0.

Fallbacks, in order: `-ts 20,28` (bias to the freer card), then `-c 8192`.
If a heavier dev1 bias still over-allocates on dev0, record the exact
`alloc_tensor_range` sizes and stop — that is a real allocator finding, not a
config knob.

### S3 — VITRIOL profile + honest report (only if S2 passes)

- `profiles/inkling-2-mini-iq4nl-16k/` modelled on an existing profile
  (`gpu.tensor_split`, `model.ngl`, `context`, `model.mmap = off`), with a
  `PROVENANCE`/comment naming the arch port commits (`74a4538`, `7ff3894`).
- Fingerprint check: launcher prints `ts=27,21`, `ngl=99`, ctx, `mmap` state.
- Measure decode t/s at 16 K filled; report the **measured** number. If it is
  below the capability floor (~8 t/s), say so plainly and label the profile
  preview-only rather than promoting it.
- Add one line to the arch-port plan's status section pointing at the result.

### S4 (conditional, only on request) — CUDA fattn head-mask rebase for `-fa on`

`inkling2.cpp` has no `cparams.flash_attn` branch, so `-fa on` is not wired.
This is real kernel work (rel-pos bias `kq_b` + sinks + SWA masks into the
fattn path) and is **out of scope unless the owner asks**; S2's `-fa off` run is
the deliverable that proves the arch runs.

## Invariants

- **I1 — 27B untouched.** No engine source edit ships in S1–S3 (they are
  launch-config and profile-only). The live 27B keeps byte-identical
  `VITRIOL-FINGERPRINT` and greedy output. Pinned by: `git diff --stat` showing
  no `engine/` changes for this work, plus a post-run 27B `/health` + slot
  restore check.
- **I2 — engine restored.** The live engine is stopped only for the model-load
  window and is restarted via `systemctl start vitriol-server.service` before the
  stage is called done. Pinned by: `systemctl is-active` == active and
  `curl /health` == ok at stage end.
- **I3 — tensor coverage.** The split run still loads 704/704 tensors, 0 missing.
  Pinned by: the load log line count (48 `VITRIOL_SUP` lines + no "missing"
  warning).
- **I4 — both cards actually used.** Gate fails if a single ≥12 GB allocation is
  attempted or if one card stays at idle memory while the other saturates. Pinned
  by: `nvidia-smi` samples captured during the run.
- **I5 — coherent text.** The continuation is real English, judged by reading it.
  Pinned by: the captured stdout in the stage log.
- **I6 — honesty.** Every number in the report is measured and traceable to a log
  file; no extrapolated t/s. Pinned by: report contents linking the logs.

## Named verification

- `s2-split-bringup` — the gate above (per-card VRAM samples + `704/704` + text).
- `s3-profile-fingerprint` — launcher prints `ts=27,21 ngl=99 mmap` off, ctx 16384.
- `i1-no-engine-diff` — `git diff --stat -- engine/` empty for this work.
- `i2-engine-restored` — `systemctl is-active vitriol-server.service` = active,
  `/health` = ok.
