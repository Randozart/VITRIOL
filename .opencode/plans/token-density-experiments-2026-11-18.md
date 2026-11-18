# Token-density & throughput experiments — full plan

**Date:** 2026-11-18 21:40
**Mode:** PLAN (research only; nothing built)
**Nature:** **EXPERIMENTAL.** Isolated worktree, own engine instance, measured, with kill
criteria. Nothing here is allowed to touch the daily driver.

---

## STATUS 2026-11-18 23:00 — harness built, E1 measured and decided

**Harness:** `density-bench/` (Rust, new crate) — 3 shapes + nested-schema shape, `baseline`
vs `grammar` arms, engine-reported timings, per-arm medians, JSON artifact, `--no-stream` for a
shared engine. 11 unit tests.

**E1 result** (`experiments/density-e1-grammar-2026-11-18.json`, 3 repeats/arm, tokens-only
because the engine was serving a live session, so timings are indicative at best):

| shape | arm | out tokens | validity | verdict |
|---|---|---|---|---|
| `structured` (flat schema) | baseline | 112 | 3/3 | — |
| `structured` | grammar | 127 (**+13.4 %**) | 3/3 | **null** — the model already complies |
| `structured-nested` (array of objects) | baseline | 533 | **0/3** | — |
| `structured-nested` | grammar | 310 (**−41.8 %**) | 3/3 | **ADOPT** |

Failure mode observed in the baseline (not a guess): the model emitted **pretty-printed** JSON and
renamed `rule` to `rule_id` — so the saving is partly whitespace, partly not inventing keys. Both
matter: one is tokens, the other is a broken contract that the harness would have had to repair.

**Consequence for the rest of the plan:** E1 is not a throughput win in general — it is a
*compliance* win that pays tokens **in proportion to schema complexity**. So the feature is
opt-in per prompt, marker-driven, and defaults **off**.

---
# Token-density & throughput experiments — full plan

**Date:** 2026-11-18 21:40
**Mode:** PLAN (research only; nothing built)
**Nature:** **EXPERIMENTAL.** Isolated worktree, own engine instance, measured, with kill
criteria. Nothing here is allowed to touch the daily driver.

---

## 0. What is already true (so we do not re-derive it)

Verified in-tree today:

| capability | where |
|---|---|
| attention-scored sparse KV eviction (**position-preserving**, no re-prefill) | `engine/src/vitriol-kv-probe.{h,cpp}`, `llama-kv-cache.cpp:3104 evict_sparse(n_needed, n_sinks)` |
| attention sinks always pinned (first `n_sinks` cells) | `llama-kv-cache.cpp:3037`, `:3130` |
| KV compression | q4_0 KV live; TurboQuant `tq3_0` certified (≈19 KiB/token on the 27B) |
| prefix cache RAM → Optane mirror, LCP slot assignment | `[server] cache_ram`, `cache_max_gb`, `slot_save_path` |
| speculative decoding | MTP `draft_n_max=2` (+31 % at depth, 2026-11-04 A/B) |
| cascade / cheap-first routing | `adaptive-router` + lapis lanes + judge model |
| harness-level context eviction | `compact-on-finish`, tool-result-clearer, `active-files` |

**Server-side grammar support already exists and needs no fork:**

- `tools/server/server-common.cpp:1168` handles `response_format`
- `common/chat.cpp:1167` builds a GBNF grammar from `json_schema`
- `tools/server/server-schema.cpp` handles tools/schema inputs

**Prompt-lookup decoding does NOT exist in the server** — only `engine/examples/lookup/`. That makes
E3 an engine task, everything else a harness task.

Baseline for judging gains (measured, 2026-11-18): prefill ~210 tok/s, decode **~1.2 tok/s at 70k+**
depth, 212k prompt tokens / 1 015 s since the last engine boot.

---

## 1. Discipline (non-negotiable)

- **I1 — daily driver untouched.** Experiments run in a git worktree with its own build tree
  (`/scratch/targets/density-<exp>`; never `/tmp`, per the linker SIGBUS rule) and its **own** engine
  on a free port with its own `slot_save_path`. The live unit, config, profiles and KV mirror are not
  edited.
- **I2 — every number carries a fingerprint.** Each engine start emits `VITRIOL-FINGERPRINT:`; each
  results row embeds the full argv + fingerprint. A result without one is void.
- **I3 — paired measurement.** Every arm is measured against a re-run baseline on the same box in the
  same session (thermal/tenant drift is the dominant noise here). Report median of 3 + spread.
- **I4 — kill criteria stated up front.** A null result is a result; record it and move on rather than
  tuning until something looks good.
- **I5 — depth honesty.** Any claim states FILLED tokens (window ≠ depth).

Worktree recipe:

```bash
git worktree add /scratch/wt/density-2026-11-18 -b exp/token-density
cd /scratch/wt/density-2026-11-18/engine
cmake -B /scratch/targets/density-engine -DCMAKE_CUDA_ARCHITECTURES="61;86" -DLLAMA_CUDA=ON
cmake --build /scratch/targets/density-engine -j 8 --target llama-server
# own instance: --port 8287 --slot-save-path /scratch/kvtier-exp --parallel 1
```

---

## 2. Measurement harness (build first, before any experiment)

Two tiers, because they answer different questions:

**Tier A — micro (direct HTTP, low variance).** Reuse the shape of `libvitriol/sweep_controller.py`
(it already spawns a server, warms up, and reports t/s). New script
`scripts/bench_density.py`:
- fixed prompt set of **three shapes** (each run cold and warm):
  1. *short structured*: "classify this tool call, emit JSON" (~120 tok in, ~40 tok out)
  2. *code edit*: realistic 8k patch request (~8k in, ~600 out)
  3. *deep recall*: 100k-token context question (~100k in, ~200 out)
- per row: TTFT, prompt tok/s, decode tok/s, **tokens until valid parse** (for structured arms),
  accepted-draft rate (when MTP is on), `prompt_tokens_total` / `prompt_seconds_total` deltas
- 3 repeats, median + min/max, all rows fingerprinted

**Tier B — macro (end-to-end through Officina).** One real turn of each shape in the worktree's TUI,
timing wall-clock to first token and to "turn settled", because harness overhead and routing can eat
a micro-win (the 2026-09-03 lesson: 40 % of decode was hidden in a dropped `[spec]` key).

---

## 3. Candidates, ranked by expected value / effort

### E1 — Grammar-constrained structured lanes *(DELIVERED, opt-in)*

> Implemented as `officina/.pi/extensions/_shared/structured-output.ts` (+ 9 tests) wired into the
> provider payload hook. A prompt asks for a constrained reply with `[[officina-json findings]]`
> (`decision` / `findings` / `report` are the declared schemas); the provider attaches
> `response_format`. `OFFICINA_STRUCTURED_OUTPUT=1` enables it — default off until a second run
> confirms, per §6. An explicit caller `response_format` always wins, and an unknown marker name
> fails open.

**Hypothesis:** the judge, decision lane, route verdicts and subagent reports are consumed
machine-first. Constraining them with `response_format: json_schema` removes both the "model rambles,
harness regexes" fragility and the forward passes spent on boilerplate.
**Method:** a payload hook in `llama-cpp-provider` (`onPayload` already exists — `progress.ts` traces
`payload.max_tokens`) that attaches `response_format` per lane, driven by a small table. No engine
change, no config change.
**Measure:** tokens-to-valid-output, TTFT, decode t/s, and end-to-end lane latency; plus a
compliance count (parses / attempts) which should go 1.00.
**Expect:** 30–60 % fewer output tokens on those lanes; ~0 change on prose lanes.
**Effort:** low. **Kill if:** valid-output token count is unchanged, or latency regresses > 3 %.

### E2 — Telegraphic + codebook protocol for internal (non-user-facing) turns
**Hypothesis:** prose filler ("I'll now…", restatement) is pure cost on turns nobody reads.
**Method:** a system-prompt layer for internal lanes only, with a fixed abbreviation table
(`R→reason1`, `A→action`, `V→verdict`) so the *harness* expands it. Measured separately from E1 so
the two do not confound.
**Measure:** output tokens per lane, task-accuracy spot check on 20 fixed cases.
**Expect:** 10–25 % output-token reduction; the risk is silent quality loss.
**Effort:** low. **Kill if:** accuracy on the 20-case set drops at all. This is a *quality* experiment
first.

### E3 — Prompt-lookup (n-gram) draft decoding *(engine work; the only big decode win left)*
**Hypothesis:** agent prompts are full of repeated paths, identifiers and snippets, so a lookup
drafter (draft from the prompt's own n-grams, verified by the main model) converts some of the 1.2 tok/s
into 2–3 tok/s without a second model or extra VRAM.
**Method:** port `engine/examples/lookup` into a draft backend behind `--spec-type lookup` next to the
existing MTP path; no change to MTP.
**Measure:** accepted-draft rate, tokens/s at 20k / 70k / 150k filled, memory delta; paired baseline.
**Expect:** large on code-edit shapes, near zero on prose. This is the only candidate that can move
the headline number.
**Effort:** **high** (engine). **Kill if:** accepted-token rate < 40 % at depth, or the drafter's
bookkeeping costs more than it saves on the deep-recall shape.

### E4 — MTP re-certification *with* grammar active
**Hypothesis:** constrained decoding makes the next token more predictable, so MTP acceptance should
rise; if it does, `draft_n_max=3` may become positive again (it was −22 % at n=3 without grammar).
**Method:** 2×2 (grammar on/off × n_max 2/3) on shape 1 and 2, at 20k and 70k filled.
**Effort:** low (config + the E1 hook). **Kill if:** acceptance does not move.

### E5 — Chinese intermediate for math-heavy steps *(honest null-result candidate)*
**Hypothesis:** the daily driver is Qwen (Chinese-heavy training), the one family where 30–40 %
token reduction in CoT has been observed.
**Method:** A/B on 15 math/algorithmic tasks: English CoT vs Chinese CoT, scoring answer correctness
first and tokens second.
**Expect:** likely null on our workloads; language switching also risks the "thinks in Chinese,
answers in Chinese" failure. **Effort:** low. **Kill if:** any correctness regression — correctness
outranks tokens absolutely.

### E6 — Eviction tuning (the shipped knob nobody has swept)

> **HARNESS READY 2026-11-18:** `density-bench sweep --engine <llama-server> --arg "<base>"
> --set score_every=8,16,32 --env VITRIOL_KV_SCORE=probe --dry-run` prints the full arm matrix
> (argv + env per arm) before anything starts; without `--dry-run` it starts one engine per arm on
> the probe URL, waits for `/health`, measures the shapes, and kills the process before the next arm
> (two engines on one GPU would measure contention, not configuration). The `deep-recall` shape now
> carries a **machine-checkable recall guard** (`expect: "cache_max_gb"`) — a sweep that loses the
> fact reports `RECALL LOST` rather than a pretty number. Verified end-to-end against a stub engine.
> **Not yet run against the real 27B: that needs a window with the daily driver stopped** (weights
> occupy both GPUs), which is exactly why the matrix is printed first.

**Hypothesis:** `score_every = 16` and the default `n_sinks` were chosen conservatively, not measured.
**Method:** sweep `score_every ∈ {8, 16, 32}` × `n_sinks ∈ {4, 32, 128}` at 70k/150k filled, measuring
`llamacpp:kv_ejected_total`, decode t/s, and a recall probe (does the model still answer a question
about a fact from 60k tokens back?).
**Expect:** a modest decode gain; the recall probe is the guard rail that stops us trading memory for
amnesia. **Effort:** low–medium (sweep script + recall set). **Kill if:** recall drops on any arm.

### Not worth trying (recorded so we stop re-litigating it)
- **New tokenizer / denser vocabulary** — we do not train the model; a new vocab invalidates every
  cached state and every weight.
- **CacheBlend-style block fusion or RoPE-hole kernels** — `evict_sparse` already does
  position-preserving eviction without re-prefill; a kernel that reorders blocks would be re-deriving
  the thing we have and risk the invariant that makes the prompt cache work.
- **Multi-model draft (separate draft model)** — MTP is in-model; a second model costs VRAM we do not
  have at 262k.

---

## 4. Order and cost

| # | experiment | effort | risk to quality | decides |
|---|---|---|---|---|
| 1 | measurement harness (Tier A) | 1 day | none | everything else |
| 2 | E1 grammar lanes | 1 day | none | whether structured lanes get cheap |
| 3 | E6 eviction sweep | 1–2 days | **recall** (guarded) | depth speed at 70k+ |
| 4 | E2 telegraphic | 1 day | **accuracy** (guarded) | output-token budget |
| 5 | E4 MTP × grammar | ½ day | none | draft depth |
| 6 | E5 Chinese CoT | ½ day | accuracy | close the question |
| 7 | E3 lookup drafter | 1–2 weeks | none | the headline decode number |

E1 first because it is the only candidate that cannot regress anything: it is per-request, harness-only,
and reversible by deleting a table entry.

## 5. What a result looks like (the table we fill in)

```
exp   shape            filled   arm                    tok/s(dec)  TTFT(s)  out-tok  recall/probe  verdict
E1    structured        1.2k     baseline                x.x         x.xx     ~54      n/a
E1    structured        1.2k     json_schema             x.x         x.xx     ~22      n/a          ?
```
Verdict rule: **adopt only if** decode or TTFT improves ≥ 3 % *and* the guard metric (valid parse,
recall, accuracy) does not regress. Anything inside noise is re-run once, then closed as null.

## 6. Merge-back policy

- E1/E2/E4 land as **opt-in extensions/config** in `officina/.pi/extensions`, default off until a
  second measurement confirms them.
- E3/E6, if they win, land in `engine/` behind a flag with the fingerprint carrying it
  (`lookup=` / `scratch=`), because a silent engine-behaviour change is exactly the drift the
  fingerprint doctrine exists to catch.
- Everything else is written up in `.opencode/plans/` and closed.

## 7. Invariants

- **I1** the daily driver's config, unit, KV mirror and installed TUI are byte-identical before and
  after every experiment (`md5sum ~/.local/bin/officina`, `git diff -- ~/.vitriol/config`).
- **I2** no result exists without a fingerprint + argv.
- **I3** a guard metric (recall / accuracy / valid-parse) is measured in the same run as any speed
  metric; a speed win with a guard regression is a loss.
- **I4** experimental branches never merge to `main` without a second, independent measurement run.
 (so we do not re-derive it)

Verified in-tree today:

| capability | where |
|---|---|
| attention-scored sparse KV eviction (**position-preserving**, no re-prefill) | `engine/src/vitriol-kv-probe.{h,cpp}`, `llama-kv-cache.cpp:3104 evict_sparse(n_needed, n_sinks)` |
| attention sinks always pinned (first `n_sinks` cells) | `llama-kv-cache.cpp:3037`, `:3130` |
| KV compression | q4_0 KV live; TurboQuant `tq3_0` certified (≈19 KiB/token on the 27B) |
| prefix cache RAM → Optane mirror, LCP slot assignment | `[server] cache_ram`, `cache_max_gb`, `slot_save_path` |
| speculative decoding | MTP `draft_n_max=2` (+31 % at depth, 2026-11-04 A/B) |
| cascade / cheap-first routing | `adaptive-router` + lapis lanes + judge model |
| harness-level context eviction | `compact-on-finish`, tool-result-clearer, `active-files` |

**Server-side grammar support already exists and needs no fork:**

- `tools/server/server-common.cpp:1168` handles `response_format`
- `common/chat.cpp:1167` builds a GBNF grammar from `json_schema`
- `tools/server/server-schema.cpp` handles tools/schema inputs

**Prompt-lookup decoding does NOT exist in the server** — only `engine/examples/lookup/`. That makes
E3 an engine task, everything else a harness task.

Baseline for judging gains (measured, 2026-11-18): prefill ~210 tok/s, decode **~1.2 tok/s at 70k+**
depth, 212k prompt tokens / 1 015 s since the last engine boot.

---

## 1. Discipline (non-negotiable)

- **I1 — daily driver untouched.** Experiments run in a git worktree with its own build tree
  (`/scratch/targets/density-<exp>`; never `/tmp`, per the linker SIGBUS rule) and its **own** engine
  on a free port with its own `slot_save_path`. The live unit, config, profiles and KV mirror are not
  edited.
- **I2 — every number carries a fingerprint.** Each engine start emits `VITRIOL-FINGERPRINT:`; each
  results row embeds the full argv + fingerprint. A result without one is void.
- **I3 — paired measurement.** Every arm is measured against a re-run baseline on the same box in the
  same session (thermal/tenant drift is the dominant noise here). Report median of 3 + spread.
- **I4 — kill criteria stated up front.** A null result is a result; record it and move on rather than
  tuning until something looks good.
- **I5 — depth honesty.** Any claim states FILLED tokens (window ≠ depth).

Worktree recipe:

```bash
git worktree add /scratch/wt/density-2026-11-18 -b exp/token-density
cd /scratch/wt/density-2026-11-18/engine
cmake -B /scratch/targets/density-engine -DCMAKE_CUDA_ARCHITECTURES="61;86" -DLLAMA_CUDA=ON
cmake --build /scratch/targets/density-engine -j 8 --target llama-server
# own instance: --port 8287 --slot-save-path /scratch/kvtier-exp --parallel 1
```

---

## 2. Measurement harness (build first, before any experiment)

Two tiers, because they answer different questions:

**Tier A — micro (direct HTTP, low variance).** Reuse the shape of `libvitriol/sweep_controller.py`
(it already spawns a server, warms up, and reports t/s). New script
`scripts/bench_density.py`:
- fixed prompt set of **three shapes** (each run cold and warm):
  1. *short structured*: "classify this tool call, emit JSON" (~120 tok in, ~40 tok out)
  2. *code edit*: realistic 8k patch request (~8k in, ~600 out)
  3. *deep recall*: 100k-token context question (~100k in, ~200 out)
- per row: TTFT, prompt tok/s, decode tok/s, **tokens until valid parse** (for structured arms),
  accepted-draft rate (when MTP is on), `prompt_tokens_total` / `prompt_seconds_total` deltas
- 3 repeats, median + min/max, all rows fingerprinted

**Tier B — macro (end-to-end through Officina).** One real turn of each shape in the worktree's TUI,
timing wall-clock to first token and to "turn settled", because harness overhead and routing can eat
a micro-win (the 2026-09-03 lesson: 40 % of decode was hidden in a dropped `[spec]` key).

---

## 3. Candidates, ranked by expected value / effort

### E1 — Grammar-constrained structured lanes *(DELIVERED, opt-in)*

> Implemented as `officina/.pi/extensions/_shared/structured-output.ts` (+ 9 tests) wired into the
> provider payload hook. A prompt asks for a constrained reply with `[[officina-json findings]]`
> (`decision` / `findings` / `report` are the declared schemas); the provider attaches
> `response_format`. `OFFICINA_STRUCTURED_OUTPUT=1` enables it — default off until a second run
> confirms, per §6. An explicit caller `response_format` always wins, and an unknown marker name
> fails open.

**Hypothesis:** the judge, decision lane, route verdicts and subagent reports are consumed
machine-first. Constraining them with `response_format: json_schema` removes both the "model rambles,
harness regexes" fragility and the forward passes spent on boilerplate.
**Method:** a payload hook in `llama-cpp-provider` (`onPayload` already exists — `progress.ts` traces
`payload.max_tokens`) that attaches `response_format` per lane, driven by a small table. No engine
change, no config change.
**Measure:** tokens-to-valid-output, TTFT, decode t/s, and end-to-end lane latency; plus a
compliance count (parses / attempts) which should go 1.00.
**Expect:** 30–60 % fewer output tokens on those lanes; ~0 change on prose lanes.
**Effort:** low. **Kill if:** valid-output token count is unchanged, or latency regresses > 3 %.

### E2 — Telegraphic + codebook protocol for internal (non-user-facing) turns
**Hypothesis:** prose filler ("I'll now…", restatement) is pure cost on turns nobody reads.
**Method:** a system-prompt layer for internal lanes only, with a fixed abbreviation table
(`R→reason1`, `A→action`, `V→verdict`) so the *harness* expands it. Measured separately from E1 so
the two do not confound.
**Measure:** output tokens per lane, task-accuracy spot check on 20 fixed cases.
**Expect:** 10–25 % output-token reduction; the risk is silent quality loss.
**Effort:** low. **Kill if:** accuracy on the 20-case set drops at all. This is a *quality* experiment
first.

### E3 — Prompt-lookup (n-gram) draft decoding *(engine work; the only big decode win left)*
**Hypothesis:** agent prompts are full of repeated paths, identifiers and snippets, so a lookup
drafter (draft from the prompt's own n-grams, verified by the main model) converts some of the 1.2 tok/s
into 2–3 tok/s without a second model or extra VRAM.
**Method:** port `engine/examples/lookup` into a draft backend behind `--spec-type lookup` next to the
existing MTP path; no change to MTP.
**Measure:** accepted-draft rate, tokens/s at 20k / 70k / 150k filled, memory delta; paired baseline.
**Expect:** large on code-edit shapes, near zero on prose. This is the only candidate that can move
the headline number.
**Effort:** **high** (engine). **Kill if:** accepted-token rate < 40 % at depth, or the drafter's
bookkeeping costs more than it saves on the deep-recall shape.

### E4 — MTP re-certification *with* grammar active
**Hypothesis:** constrained decoding makes the next token more predictable, so MTP acceptance should
rise; if it does, `draft_n_max=3` may become positive again (it was −22 % at n=3 without grammar).
**Method:** 2×2 (grammar on/off × n_max 2/3) on shape 1 and 2, at 20k and 70k filled.
**Effort:** low (config + the E1 hook). **Kill if:** acceptance does not move.

### E5 — Chinese intermediate for math-heavy steps *(honest null-result candidate)*
**Hypothesis:** the daily driver is Qwen (Chinese-heavy training), the one family where 30–40 %
token reduction in CoT has been observed.
**Method:** A/B on 15 math/algorithmic tasks: English CoT vs Chinese CoT, scoring answer correctness
first and tokens second.
**Expect:** likely null on our workloads; language switching also risks the "thinks in Chinese,
answers in Chinese" failure. **Effort:** low. **Kill if:** any correctness regression — correctness
outranks tokens absolutely.

### E6 — Eviction tuning (the shipped knob nobody has swept)

> **HARNESS READY 2026-11-18:** `density-bench sweep --engine <llama-server> --arg "<base>"
> --set score_every=8,16,32 --env VITRIOL_KV_SCORE=probe --dry-run` prints the full arm matrix
> (argv + env per arm) before anything starts; without `--dry-run` it starts one engine per arm on
> the probe URL, waits for `/health`, measures the shapes, and kills the process before the next arm
> (two engines on one GPU would measure contention, not configuration). The `deep-recall` shape now
> carries a **machine-checkable recall guard** (`expect: "cache_max_gb"`) — a sweep that loses the
> fact reports `RECALL LOST` rather than a pretty number. Verified end-to-end against a stub engine.
> **Not yet run against the real 27B: that needs a window with the daily driver stopped** (weights
> occupy both GPUs), which is exactly why the matrix is printed first.

**Hypothesis:** `score_every = 16` and the default `n_sinks` were chosen conservatively, not measured.
**Method:** sweep `score_every ∈ {8, 16, 32}` × `n_sinks ∈ {4, 32, 128}` at 70k/150k filled, measuring
`llamacpp:kv_ejected_total`, decode t/s, and a recall probe (does the model still answer a question
about a fact from 60k tokens back?).
**Expect:** a modest decode gain; the recall probe is the guard rail that stops us trading memory for
amnesia. **Effort:** low–medium (sweep script + recall set). **Kill if:** recall drops on any arm.

### Not worth trying (recorded so we stop re-litigating it)
- **New tokenizer / denser vocabulary** — we do not train the model; a new vocab invalidates every
  cached state and every weight.
- **CacheBlend-style block fusion or RoPE-hole kernels** — `evict_sparse` already does
  position-preserving eviction without re-prefill; a kernel that reorders blocks would be re-deriving
  the thing we have and risk the invariant that makes the prompt cache work.
- **Multi-model draft (separate draft model)** — MTP is in-model; a second model costs VRAM we do not
  have at 262k.

---

## 4. Order and cost

| # | experiment | effort | risk to quality | decides |
|---|---|---|---|---|
| 1 | measurement harness (Tier A) | 1 day | none | everything else |
| 2 | E1 grammar lanes | 1 day | none | whether structured lanes get cheap |
| 3 | E6 eviction sweep | 1–2 days | **recall** (guarded) | depth speed at 70k+ |
| 4 | E2 telegraphic | 1 day | **accuracy** (guarded) | output-token budget |
| 5 | E4 MTP × grammar | ½ day | none | draft depth |
| 6 | E5 Chinese CoT | ½ day | accuracy | close the question |
| 7 | E3 lookup drafter | 1–2 weeks | none | the headline decode number |

E1 first because it is the only candidate that cannot regress anything: it is per-request, harness-only,
and reversible by deleting a table entry.

## 5. What a result looks like (the table we fill in)

```
exp   shape            filled   arm                    tok/s(dec)  TTFT(s)  out-tok  recall/probe  verdict
E1    structured        1.2k     baseline                x.x         x.xx     ~54      n/a
E1    structured        1.2k     json_schema             x.x         x.xx     ~22      n/a          ?
```
Verdict rule: **adopt only if** decode or TTFT improves ≥ 3 % *and* the guard metric (valid parse,
recall, accuracy) does not regress. Anything inside noise is re-run once, then closed as null.

## 6. Merge-back policy

- E1/E2/E4 land as **opt-in extensions/config** in `officina/.pi/extensions`, default off until a
  second measurement confirms them.
- E3/E6, if they win, land in `engine/` behind a flag with the fingerprint carrying it
  (`lookup=` / `scratch=`), because a silent engine-behaviour change is exactly the drift the
  fingerprint doctrine exists to catch.
- Everything else is written up in `.opencode/plans/` and closed.

## 7. Invariants

- **I1** the daily driver's config, unit, KV mirror and installed TUI are byte-identical before and
  after every experiment (`md5sum ~/.local/bin/officina`, `git diff -- ~/.vitriol/config`).
- **I2** no result exists without a fingerprint + argv.
- **I3** a guard metric (recall / accuracy / valid-parse) is measured in the same run as any speed
  metric; a speed win with a guard regression is a loss.
- **I4** experimental branches never merge to `main` without a second, independent measurement run.
