# Kintsugi-27B trial on this host

2026-11-24. Owner requested a hands-on trial of `CaryPalmer/Kintsugi-27B`
(HF, Apache-2.0, 10.5 GB). Research: it is Qwen3.8-27B at 2.47 bits/weight
(EXL3 trellis codec — L0xRE-V2 bit allocation + Escha W2 + exllamav3-style
kernels), full 262,144-token context on one 12 GB card via tiered q4_0 KV,
MTP×3 + lookup drafting built in, optional DFlash2 block drafter (1.1 GB,
Q4_0, Apache-2.0). Their benchmarks: AppWorld 294/336 vs BF16 293/336
(fresh seeds, p = 1.0); their own RTX 3060 12 GB report: 41.5 / 33.2 /
31.6 / 25.6 / 16.8 t/s at short/64k/117k/200k/245k, 11.6 GB VRAM, no spill.
Stock llama.cpp cannot load the file; the trial runs their prebuilt
v1.1.6 Linux engine as a *second, stoppable* service.

## Trial design

- Deploy to `~/kintsugi/` (bundle tarball + models/). Nothing under the
  VITRIOL tree is modified; the 27B/Inkling services are untouched files.
- Kintsugi runs **only while `vitriol-server.service` is stopped** — one
  engine per card. Restore the 27B service before the trial is over.
- Card choice: `CUDA_VISIBLE_DEVICES` pinned per run; dev0 first
  (12 GB, ~700 MiB desktop). Port 8290.
- Drafter A/B: default (MTP + lookup) vs DFlash2 drafter file present.

## Gates / tests

1. **G1 — integrity**: `sha256sum` of `Kintsugi-27B-12G.gguf`
   == `0510bae82445dc53dbc7d0d842d8b7e146b8eafce0837e39b157d411e48c8a74`
   (README, cross-checked against the tree API `lfs.oid`); drafter
   (if downloaded) == `0a994382adf17c720c9e5d4292f57344f3b445f058d3e9d5b0f601ae7d6ca3c4`.
2. **G2 — engine starts**: their `start-server.sh` (or the engine binary
   directly) reaches `/health`; log must show the EXL3 model loaded and
   the tiered KV sizes.
3. **G3 — quality probes** (thinking off, temp 0):
   - P1 short prompt, 250 max tokens: coherent, tool-calling shape
     uncorrupted (our lane would send the same shapes).
   - P2 4k prefill: decode t/s;
   - P3 64k prefill: prefill t/s + decode t/s (compare their 33.2 @ 64k).
   - P4 needle: plant a fact at ~60k, ask at the end (their 24/24 claim).
   - P5 identical-resend: send P1 twice — second must be prefix-cached
     (their restore test analog).
4. **G4 — drafter A/B**: P2/P3 with and without the DFlash2 file;
   record draft acceptance and t/s (they claim +25–35 %).
5. **G5 — restore**: `systemctl restart vitriol-server.service`,
   `/health` ok, `local-ok`, fingerprint line present and matching the
   qwen38 profile; `nvidia-smi` no leftover Kintsugi process.

## Invariants

- **I1 (parity of the daily driver)** — after the trial, the 27B service
  is back, fingerprint-diffed against the pre-trial capture. *Pinned by* G5.
- **I2 (provenance)** — no GGUF is run without a verified sha256 against
  the tree API `lfs.oid`. *Pinned by* G1.
- **I3 (no overlap)** — Kintsugi runs only while the VITRIOL service is
  stopped; both engines never share a card. *Pinned by* the stop/start
  ordering in G2/G5 and `nvidia-smi --query-compute-apps`.
- **I4 (no config drift)** — `~/.vitriol/config` and all profiles are
  byte-identical before/after (copy taken in advance; md5 diff at the end).
  *Pinned by* G5.
- **I5 (reversibility)** — the trial installs nothing into PATH and
  writes nothing above `~/kintsugi/`; deleting the directory removes it
  entirely.

## Numbers to record

| probe | Kintsugi here | their 3060 report | 27B daily driver |
|---|---|---|---|
| short decode | ? | 41.5 t/s | 16.49 t/s baseline |
| 64k prefill/decode | ? | ? / 33.2 t/s | 19.7 prefill @ 3.9k (Inkling) |
| 64k decode | ? | 33.2 t/s | ~12.7 t/s measured |
| needle @60k | ? | 24/24 @ 245k | — |
| DFlash2 gain | ? | +25–35 % | — |

## Follow-up stage (only if the trial earns it)

Port the EXL3 trellis kernels (upstream PR #32381 + their two-stage head /
lookup drafting) into the frozen VITRIOL engine, Inkling-style patch-by-
patch with `PROVENANCE:` notes, so Kintsugi can run under our launcher
with KV caps, slots, ctx-resize and the Lapis lane. Separate plan.
