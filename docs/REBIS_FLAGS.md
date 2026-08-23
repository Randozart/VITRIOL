# REBIS flag reference — every knob, what it does, why this value

Ground truth for every flag in active use across the REBIS stack. Semantics
cross-checked against `llama.cpp/common/arg.cpp` registrations; consequences
measured on this rig (RTX 3060 12 GB + GTX 1070 Ti 8 GB + 15 GB RAM) unless
marked estimated. Canonical launcher: `scripts/rebis-servers.sh`.

PROVENANCE: original documentation — flags registered in llama.cpp arg.cpp;
measurements from EXPERIMENT_LOG.md entries dated 2026-08-21/22.

## Head server flags (Sol :8279 / Luna :8247)

### Model & placement

| flag | our value | does | why |
|---|---|---|---|
| `-m PATH` | per-head GGUF | model file | Sol: UD-IQ2_S 27B; Luna: Mellum2 IQ4_XS 12B-A2.5B |
| `-ngl 99` | all layers on GPU | layers offloaded to VRAM | heads are fully resident; `99` ≈ "everything" |
| `-c N` | 65536 | context window in tokens | hermes demands ≥64k; measured KV fits both cards at q4_0 |
| `--host 127.0.0.1 --port P` | loopback only | bind address | never expose inference to the network |

### KV cache & attention

| flag | our value | does | wrong-value failure |
|---|---|---|---|
| `--cache-type-k q4_0` | quantized K cache | 4-bit K vectors: ~4× smaller KV | omit → fp16 KV doubles/triples VRAM; quality loss negligible for chat/code |
| `--cache-type-v q4_0` | quantized V cache | same for V | requires `-fa on`; without FA, v-quant silently ignored upstream |
| `-fa on` | flash attention | fused attention kernels; prerequisite for v-cache quant | off → v q4_0 ignored, slower prefill |
| `--context-shift` | rolling window | when prompt would exceed ctx, drop oldest span and continue instead of erroring | absent → hard overflow error mid-long-session |
| `--cache-reuse 256` | chunked reuse | re-evaluate removed/shifted regions in ≥256-token chunks via KV shift | large values thrash; 0 disables chunk reuse |

**Measured**: rolling+reuse restored post-H1 gate; long-session sim ran
22/22 turns past 64k with zero overflow errors.

### Prompt cache & checkpoints (VITRIOL layer)

These are fork additions over upstream llama.cpp.

| flag | our value | does | failure mode if misused |
|---|---|---|---|
| `--cache-ram N` | 2048 (Sol) / 1024 (Luna) MiB | caps the semantic prompt cache that stores full conversation states in host RAM | **default 8192 unbounded-ish → host OOM kills** (incident, 2026-08-21); 0 disables entirely |
| `--slot-prompt-similarity F` | 0.1 default; we ran 0 during isolation | slot dispatch picks the slot whose cached prompt best matches the new one | low threshold + ungated restore caused **cross-session bleed**; H1 gate (`--prompt-cache-min-lcp`) now guards the restore itself |
| `--prompt-cache-min-lcp F` | 0.5 (H1 default) | refuse restoring any saved state whose longest-common-prefix share is below F | without it: unrelated prompts inherit foreign context |
| `--ctx-checkpoints N` | 12 (upstream 32) | max saved mid-prefill checkpoints per slot (~150 MB each at large ctx) | 32 × big contexts = multi-GB host RAM creep over day-long sessions |
| `--checkpoint-every-n-tokens N` | 8192 (upstream 2048) | checkpoint spacing during prefill | tighter spacing = more checkpoints = more RAM |

### Server surface

| flag | does | note |
|---|---|---|
| `--slots` | enables `GET /slots` endpoint | TUI progress bars + gateway introspection depend on it |
| `--metrics` | enables `GET /metrics` (prometheus) | token totals, throughput gauges |
| `--jinja` | apply the model's chat template to `/v1/chat/completions` | required for Thinking/instruct template correctness; also enables `/apply-template` used by anticipatio warming |
| `--host/--port` | bind | loopback-only by policy |

## Sampling & thinking controls (per-request)

| field | our value | effect |
|---|---|---|
| `temperature` | 0.6 drafter / 0.2 verifier / 0.0 constrained verdicts | JetBrains recommendation at 0.6; audits want determinism |
| `top_p` / `top_k` | 0.95 / 20 | nucleus+top-k trim; setdefaults in shim when client omits |
| `chat_template_kwargs.enable_thinking` | false for delta drafting | **off: deltas come out clean; on: 40–70% of budget burns pre-diff and output degrades into repetition loops** (measured) |
| `max_tokens` | 4096 file / 8192 patch / ≥2048 audits | floor must cover draft + think; too small ⇒ empty content or truncated verdicts |

## Mercury gateway flags (:8280)

| flag | default | does |
|---|---|---|
| `--mode gateway\|steer\|passthrough` | gateway | gateway = route ladder + audit pipeline; steer = legacy watch-only; passthrough = dumb proxy |
| `--luna-url` / `--sol-url` | :8247 / :8279 | backend heads |
| `--luna-model` / `--sol-model` | GGUF ids | ids forwarded on backend calls |
| `--distill-dir` | ~/.vitriol/distill | capture store |
| route ladder | see below | kickoff→Sol · tool-result continuation→Luna · finalizing→pipeline · no-tools→Sol (quality-first) |

## Mandatum loop flags (`rebis.py`)

| flag | default | does |
|---|---|---|
| `--task FILE` | — | Mandatum packet path |
| `--drafter-url/--verifier-url` | Luna/Sol | head endpoints |
| `--mode rebis\|baseline` | rebis | baseline = single-shot big-model control arm |
| `--resume TASK_ID` | — | continue a journaled run |
| `--budget-s N` | none | wall-clock ceiling; aborts pause resumably |
| `--report FILE` | — | write run report JSON (tokens, iterations, verdicts) |
| `--drafter-spawn/--verifier-spawn` | none | respawn commands when a head dies |
| `--no-distill` / `--distill-dir` | on / ~/.vitriol/distill | training-data capture |
| `--anticipatio` | off | shadow-prefill stable prefix after each send |

Packet keys: `draft_mode` file|patch|replace · `verify_mode`
compiler_only|llm · `draft_budget` token cap · `max_iterations`.

Protocol selection law: whole-file ≤~250 lines; replace-mode for real-file
modifications (verbatim SEARCH anchors); patch-mode only with verbatim-
context discipline. See REBIS-GUIDE §0 drafter matrix.
