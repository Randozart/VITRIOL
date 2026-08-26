# GLOSSARY — how VITRIOL talks

Alchemical names are load-bearing. This is the translation layer.

## Core concepts

| term | meaning |
|---|---|
| **Residency rule** | weights live in VRAM when they fit; streaming/DMA offload only when combined VRAM is genuinely exceeded (`VITRIOL_MODE=off` default) |
| **Window ≠ depth** | an allocated context window says nothing about usable filled-context depth; only filled-token benchmarks count |
| **Filled-depth certification** | benchmark methodology: chunked or single-shot prefill to N tokens, then decode at depth — the only accepted evidence for context claims |
| **Slot tenancy** | one llama-server hosts multiple tenants on split windows (`--slot-context 0=73728,1=8192`); tenants own their slot by contract |
| **Generation change** | a new llama-server PID; triggers checkpoint replay by the sidecar |

## Subsystem names

| name | etymology / role |
|---|---|
| **VITRIOL** | `Visita Interiora Terrae Rectificando Invenies Occultum Lapidem` — the project |
| **LULL** | attention-probe KV scoring + eviction + pool-reset (the quiet period before depth runs out) |
| **TurboQuant (tq3_0/tq3_1s/tq3_4s)** | 3.5 bpw KV cache quantization types |
| **RAM Shot** | ⚰️ page-locked host memory serving MoE experts (35B era; see VERDICTS) |
| **Lapis Occultus** | "the hidden stone" — server alias for the Qwen3.8-27B production model |
| **Hermetis** | memory system (session search, episodic store) |
| **Rebis** | dual-model cognitive architecture ("res bina" — two heads, one silicon) |
| **Officina** | interactive workshop UI |
| **Spagyric** | hardware autotuner (dissolve, purify, recombine — profiles) |
| **Copula** | opencode ↔ VITRIOL bond plugin |
| **Pymander** | curated reference mind (divine Pymander, Hermetica corpus) |
| **ontic forge** | second tenant on slot 1; generation/curation workload |
| **alka** | ⚰️ legacy kernel-instruction DSL from the DMA era |

## Runtime vocabulary

| term | meaning |
|---|---|
| **Fingerprint** | `VITRIOL-FINGERPRINT:` line emitted at every launch — launcher, server argv, runners; flag provenance is mandatory |
| **Sidecar** | `lull_slot_persist.py` — restore, autosave, hang watchdog, proactive bounce, oom-shield |
| **Bounce** | scheduled checkpointed restart before a predicted wedge (proactive) or after a hang (reactive) |
| **Sticky stop** | `vitriol stop` via systemctl: stays down until explicitly started again |
| **oom-shield** | raises big consumers' `oom_score_adj` so the kernel prefers them over the server |
| **Clobber guard** | empty saves never replace rich checkpoints (tmp-file staging) |
| **Churn guard** | skip autosave ticks when global activity counters are frozen |
