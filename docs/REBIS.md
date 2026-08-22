# REBIS — dual-model cognitive architecture

*res bina — one agent, two heads, one body of silicon.*

REBIS unifies two local models into a single inference endpoint: **Sol**
(Qwen3.8-27B, deep reasoning) and **Luna** (Mellum2-12B MoE, high-velocity
drafting), orchestrated so each head's weakness is masked by the other's
strength. Built for asymmetric commodity hardware — an RTX 3060 12 GB and a
GTX 1070 Ti 8 GB on a 15 GB RAM box.

---

## 1. Vision

> Point hermes-agent at a single endpoint and get fast, yet verified
> responses. Qwen3.8 is very smart but exceedingly slow on this hardware;
> Mellum2 is fast but lacks initiative. REBIS makes them work in unison.

Concretely: planning and judgment run on Sol at full depth; code generation
and mechanical turns run on Luna at 70 tok/s; substantive answers are audited
by Sol before delivery; every interaction is captured as training data.

## 2. Topology

```
hermes / opencode ──► MERCURY :8280   (gateway, Hg=80)
                         │ route ladder
      ┌──────────────────┼──────────────────┐
      ▼ reason/kickoff   ▼ executor         ▼ finals + flagged
  SOL :8279          LUNA :8247        LUNA drafts ∥ SOL ingests
  (Au=79, Qwen3.8)   (Ag=47, Mellum2)  → constrained verdict →
  full-depth reason  70 tok/s drafting    pass | Sol correction
```

Port names encode alchemical metals by atomic number: Aurum 79, Argentum 47,
Hydrargyrum 80. Mercury is both the alchemical mediator and Hermes' Roman
name — the messenger standing between the two heads.

| component | endpoint | model |
|---|---|---|
| Sol | `:8279` | Qwen3.8-27B UD-IQ2_S, resident GPU0, 64k ctx |
| Luna | `:8247` | Mellum2-12B-A2.5B-Thinking i1-IQ4_XS, pinned GPU1, 64k ctx |
| Mercury | `:8280` | advertised id `rebis` (+ `rebis-qwen`, `rebis-mellum` escape hatches) |

Measured performance (see §8): Sol ~20 tok/s decode / ~430 tok/s prefill;
Luna **70 tok/s** decode / ~557 tok/s prefill, fully pinned at 6.87/8 GiB.

## 3. Components

| path | role |
|---|---|
| `libvitriol/rebis_shim.py` | Mercury gateway: turn classifier, draft-audit pipeline, anticipatio warming, compaction, distill capture |
| `libvitriol/rebis.py` | Mandatum loop controller (implementation-scale tasks), journal/resume, distill capture |
| `libvitriol/prefill_probe.py` | ingestion/reuse measurement against any head |
| `libvitriol/long_session_sim.py` | day-long session simulator (>64k validation) |
| `scripts/rebis-servers.sh` | canonical head launcher (supervised modes) |
| `scripts/launch_vitriol_full.sh` | legacy TUI-stack launcher |
| `vitriol-tui/` | observability: dual-GPU panels, per-slot progress bars |
| `llama.cpp` (fork) | gated prompt cache (`--prompt-cache-min-lcp`), VITRIOL checkpoints/DMA |
| `docs/REBIS-GUIDE` rules → `libvitriol/REBIS-GUIDE.md` | Mandatum authoring law |

### 3.1 Gateway routing ladder

Evaluated per `/v1/chat/completions` turn (escape-hatch model ids win first):

1. tools attached + no assistant tool activity yet → **Sol** (planner authors
   first calls; catches the drafter's under-initiation failure class)
2. last message is a tool result → **Luna** fast path (executor continuation)
3. assistant finalizing after tool work → **pipeline** (draft-audit)
4. plain chat: complexity heuristic (length/design markers) → Sol or Luna
5. fallback → Sol (quality over speed)

Pipeline detail: Luna drafts (thinking off — deliberation burns budget the
delta needs); a one-shot anticipatio warm feeds `stable prefix + draft` into
Sol's cache concurrently; a constrained JSON verdict (`complete`,
`missing_actions`) gates the answer; on failure Sol authors the correction
natively in OAI tool-call format.

### 3.2 Mandatum loop (`rebis.py`)

For implementation-scale tasks invoked as a tool:

```
Mandatum packet {objective, invariants I1..In, file_slices,
                 compile_cmd, max_iterations}
  → Luna/Qwen drafts (file | patch | replace protocols)
  → fragment guard + .rebis-bak snapshots
  → compiler gate (ERROR DIGEST fed back on failure)
  → verdict {pass, checks[{id, holds, evidence}]}  (or compiler_only mode)
  → poke with correction orders (drafter sees its own last draft)
  → ≤max_iterations or wall-clock budget ⇒ pause/resume via journal
```

Verification modes: `compiler_only` when the gate enforces every invariant
(test-emitting invariants make this the norm); `llm` adds Sol's evidence-or-
fail audit for semantic invariants compilers cannot see.

### 3.3 Day-long memory

- Rolling windows restored post-H1 (`--context-shift --cache-reuse 256`)
- Gateway **compaction**: history >48k tokens ⇒ Sol writes a SESSION MEMORY
  digest (paths/outcomes/exit-codes verbatim), spliced as a system message;
  recent ~10k tokens stay verbatim; recursion-guarded
- Supervised server modes auto-respawn after co-tenant killalls (15 s backoff)
- Checkpoint RAM bounded: `-ctx-checkpoints 12 -cpent 8192`

## 4. Operations

### 4.1 Start / stop

```bash
./scripts/rebis-servers.sh both-sup     # supervised heads (auto-respawn)
python3 libvitriol/rebis_shim.py --port 8280   # gateway
```

Supervised heads log exits to `/tmp/rebis-supervise.log`; gateway to
`/tmp/shim.log`. Head logs: `/tmp/qwen.log`, `/tmp/mellum.log`.

### 4.2 Wire hermes

```yaml
- name: REBIS-GATEWAY
  base_url: http://127.0.0.1:8280/v1
  model: rebis
  models:
    rebis:
      context_length: 65536
```

opencode: provider `mellum-think` points at Luna directly (`:8247/v1`);
Qwen profiles unchanged.

### 4.3 Loop invocation (from hermes brain or shell)

```bash
python3 libvitriol/rebis.py --task task.json \
    --drafter-url http://127.0.0.1:8247 \
    --verifier-url http://127.0.0.1:8279 \
    --budget-s 600 --report out.json [--resume TASK_ID] [--anticipatio]
```

Packet authoring law lives in `libvitriol/REBIS-GUIDE.md` (jointly
satisfiable invariants, test-emitting invariants by default, slice hygiene).

### 4.4 Key knobs

| knob | default | note |
|---|---|---|
| `--cache-ram` | 2048/1024 | **always set** — unbounded default OOMs on 15 GB boxes |
| `--ctx-checkpoints` | 12 here (32 upstream) | checkpoint RAM bound |
| `--slot-prompt-similarity` | 0.1 | raise if unrelated sessions cross-contaminate |
| `--compact-threshold` | 48000 tok | gateway compaction trigger |
| `enable_thinking:false` | drafter deltas + corrections | thinking burns budget without improving deltas |

## 5. Distillation pipeline (D1 live, D2 deferred)

Every loop run, baseline shot, gateway-audited turn, steering intervention,
and compaction appends JSONL to `~/.vitriol/distill/`. Records contain full
drafter texts, before/after file snapshots, verdicts, token spend — rejected
iterations included (the dispreferred side of preference pairs).

**Local-only policy**: records embed repo code. Never commit or sync.

Conversion to SFT chat samples / DPO pairs is deliberately deferred. When
volume justifies compute: base =
[`JetBrains/Mellum2-12B-A2.5B-Thinking-SFT`](https://huggingface.co/JetBrains/Mellum2-12B-A2.5B-Thinking-SFT)
(their sanctioned pre-RL checkpoint); local training blocked until Unsloth
gains `mellum` arch support (MoE QLoRA unsupported anywhere today; bf16 LoRA
needs ~63 GB).

## 6. Troubleshooting

| symptom | cause | fix |
|---|---|---|
| servers vanish when idle | co-tenant runs `killall llama-server`; shared build dir binaries deleted | supervised modes + private `build-rebis`; coordinate with tenant |
| OOM kills on llama-server | unbounded `--cache-ram` or `--no-mmap` staging collision on 15 GB RAM | bounded cache-ram; mmap weights; stagger starts |
| unrelated content leaks into answers | pre-cache-gate restore bug | fixed by min-LCP gate; keep H1 binary |
| empty drafter responses | Thinking variant exhausted budget inside `<think>` | bigger `draft_budget`, or `enable_thinking:false` |
| `pkill` kills your own shell | pattern matched own argv | `[x]` bracket trick |
| 502 bursts through gateway | supervisor respawn bind-race | self-heals; backoff now 15 s |
| latency grows over a long session | history growth + co-tenant eviction | compaction fires at 48k; dedicated endpoints for concurrent clients |

## 7. Provenance & licensing

All REBIS orchestration code is original (GPL-2.0, this repo). Mellum2 weights
are Apache-2.0 (JetBrains); running them as separate server processes imposes
no GPL obligation on VITRIOL. The D2 fine-tune base
(`Mellum2-Thinking-SFT`) inherits Apache-2.0. No third-party code was copied
into this system; design patterns were studied, not transplanted.

## 8. Measured performance summary

| metric | value | condition |
|---|---|---|
| Luna decode (pinned) | 69.8–70.2 tok/s | 64k ctx, IQ4_XS, 1070 Ti |
| Luna prefill | 442–559 tok/s | same |
| Sol decode (IQ2_S resident) | 19.6–20.4 tok/s | 32k–64k ctx, 3060 |
| Sol prefill | 394–438 tok/s | vs 239–264 streaming baseline |
| Prefix reuse | 46.95 s → 0.06 s | identical 19.8k prompt, gated cache |
| Rebis loop vs direct | 130 s vs 18m23s | same task, equal quality gate |
| Long-session endurance | 22/22 turns past 64k | simulator, 3 compactions |

Full experiment record: `EXPERIMENT_LOG.md`; phase plans:
`.opencode/plans/rebis-*.md`.
