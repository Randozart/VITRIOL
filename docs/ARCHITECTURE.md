# VITRIOL Architecture

> **Status**: living document, reflects behavior as of 2026-08-26.
> History lives in [`archive/`](archive/); dead ideas in [VERDICTS.md](VERDICTS.md).

## Thesis

Run large MoE models on a VRAM-constrained, DDR3-era desktop as a **resident,
durable appliance** — not as a process that dies with the first memory crisis.
The model weights live in VRAM; every other resource (host RAM, zram, PCIe
bandwidth, the server's task queue) is scarce and must be budgeted explicitly.

## The stack, one screen

```
┌─────────────────────────────────────────────────────────────┐
│ TENANTS                                                     │
│   hermes-agent (CLI/gateway)   ontic forge   Rebis gateway  │
│        │  slot 0 (73k)            │ slot 1 (8k)   │         │
├────────┼──────────────────────────┼───────────────┼─────────┤
│ ENGINE │ llama.cpp fork ("vitriol" branch)                  │
│        │  - residency rule: weights VRAM-resident,          │
│        │    streaming/DMA offload rejected by measurement   │
│        │  - LULL: attention-probe KV scoring, eviction,     │
│        │    pool-reset (+~20% usable depth)                 │
│        │  - TurboQuant KV: tq3_0/tq3_1s/tq3_4s (3.5 bpw)    │
│        │  - --slot-context per-slot sizing + capacity-aware │
│        │    routing                                         │
│        │  - --slot-save-path sequence checkpoints           │
│        │  - MTP draft head / GDN arch support               │
├────────┼────────────────────────────────────────────────────┤
│ RUNTIME│ scripts/vitriol launcher                           │
│        │  profiles → argv, flag fingerprint at every launch │
│        │ systemd user units                                 │
│        │  vitriol-server.service    Restart=always, sticky  │
│        │  vitriol-autosave.service  persistence sidecar     │
│        │    startup restore · autosave w/ churn guard       │
│        │    clobber protection · oom-shield                 │
│        │    hang watchdog · proactive bounce                │
├────────┼────────────────────────────────────────────────────┤
│ TRUTH  │ libvitriol (Rust calibrator, GGUF-derived VRAM     │
│        │ math, zero hardcoded models)                       │
│        │ certification reports: filled-depth benchmarks     │
│        │ only; shallow-bench numbers do not count           │
└────────┴────────────────────────────────────────────────────┘
```

## Engine principles

### Residency rule
Stream/DMA offloading only when weights exceed combined VRAM. For resident-
capable quants the default is `VITRIOL_MODE=off`. Streaming a fitting model
starves the GPUs on DDR3/PCIe expert fetches — measured pessimization, not
premature optimization. See VERDICTS.md.

### Window ≠ depth
A context window allocation says nothing about usable filled-context depth.
Every capability claim must carry a *filled* token count from a chunked or
single-shot prefill plus decode-at-depth. Certification reports live in
`.opencode/plans/` and `docs/BENCHMARKS.md`.

### KV economics
KV cache is priced in bits-per-weight and scored for value:
`tq3_0` TurboQuant ≈ 3.5 bpw (−22% vs q4_0), per-device overrides via
`VITRIOL_KV_QUANT[_K|_V]_GPU<d>`; LULL probe scoring decides which KV pages
earn their keep; pool-reset rewinds compute pools to recover ~20% depth.

### Slot tenancy
One server hosts multiple tenants on split context windows
(`--slot-context 0=73728,1=8192`). Routing is capacity-aware: a prompt never
lands on a slot too small for it, and LRU ties resolve to the lowest slot id.
Tenant contracts are explicit — e.g. ontic pins `id_slot: 1`.

## Durability model

The only restart-surviving state is disk checkpoints written through
`--slot-save-path` (`slot{N}.bin`, ~150 MiB base for GDN recurrent state +
KV). Therefore:

1. On any new server PID, the sidecar replays `slot{N}.bin` into slot N.
2. Periodic autosave skips ticks when global activity counters prove nothing
   changed, and stages writes through `slotN.tmp.bin` so an empty save can
   never clobber a rich checkpoint (`--cache-idle-slots` makes occupied slots
   look empty).
3. Client-side conversation history (hermes) is the source of truth for
   *content*; checkpoints restore *warmth*, not messages.

## Runtime guarantees

| failure | response | bound |
|---|---|---|
| server exit (OOM kill, crash) | systemd `Restart=always` + sidecar replay | ~40 s |
| server hang (health-deaf, same PID) | sidecar hang watchdog forces restart after ~60 s | ~90 s |
| sustained host memory exhaustion | proactive bounce: checkpointed clean restart before the wedge | ~2 min detection |
| operator stop | `vitriol stop` is sticky (systemctl-based); no resurrection | n/a |

All of this is implemented in `scripts/lull_slot_persist.py` +
`systemd/user/*.service`; knobs documented in [OPERATIONS.md](OPERATIONS.md).

## Provenance discipline

Every launch emits a `VITRIOL-FINGERPRINT:` line (launcher, server main,
runners); every benchmark result embeds full argv. Silent flag drift is a
review blocker. Licensing is Apache-2.0 (since 2026-08-28; previously GPL-2.0);
third-party code enters only via the
inspiration/copy rules in `docs/provenance/`.

## Repository layout

```
llama.cpp/          submodule, "vitriol" branch = canonical daily driver
scripts/vitriol     launcher: config/profiles/serve/bench/config mgmt
scripts/lull_slot_persist.py   persistence + watchdog sidecar
libvitriol/         Rust calibrator (GGUF parser, VRAM estimator, CLI)
profiles/           canonical configs (personal + examples/)
systemd/user/       unit files
docs/               living documentation (this directory)
.opencode/plans/    agent session reports — the raw lab notebook
```
