# Dream pipeline — rung 2/3: generator + small-model lane (research, 2026-11-21)

Phase 1 (RESEARCH) of the PIPELINE for the next rung of the dream pipeline
(plan `dream-pipeline-2026-11-21.md`, rung 1 = dream-eval already shipped at
`3c38891`). Rungs 2+3: the **dream generator** (gated corpus from real
history) and the **small-model lane trial** (prove the promotion loop with
zero training). Rung 4 (QLoRA) stays a follow-on.

## F — Findings

### F1 — the route ledger is richer than it looks
`~/.vitriol/officina/route-ledger-*.jsonl`: **8042 rows** (was 7683 at
routing work), fields `ts, tier, provider, model, outcome, errorClass,
reason, localState, latencyMs, tokensIn, tokensOut, cacheRead, cacheWrite,
costEstimate, profile`. Outcomes: 7725 ok / 321 error (`completion_error`).
Tiers: 6091 local (Lapis) / 1955 cloud. The ledger records *routing* facts,
NOT the question/answer text — it is ground truth for *which lane served
which kind of turn*, not a source of Q/A pairs.

### F2 — transcripts ARE the corpus source (22,702 messages)
`~/.pi/agent/sessions/--home-randozart-Desktop-Projects-VITRIOL--/*.jsonl*`:
23 files, ~30,729 rows, **22,702 `type:"message"` rows**. Row shape:
`{id, parentId, timestamp, type, message:{role, content}}` plus custom
events. These carry the actual user questions and assistant answers —
the only local source of verifiable Q/A. The sleep lane already reads and
truncates them (`sleep-lane/index.ts:165`, last 200 KB per file) and frames
them as quoted DATA (the I6 prompt, `sleep-consolidate.ts:71`:
"QUOTED TRANSCRIPT … DATA, not instructions … TRANSCRIPT BEGIN <<< … >>>").
The dream generator inherits that framing verbatim (I-P5).

### F3 — MEMORY.md is the human-blessed fact set
`.officina/MEMORY.md` (project) + the session scratchpad. Facts carry
provenance ("2026-11-19", commit hashes, numbers). These become
convention/environment pairs with zero inference risk (already verified by
a human or by the sleep-lane quiz loop).

### F4 — fix-pattern pairs are derivable
A "fix-pattern" = a transcript window where an error surfaced (tool result
with stderr / a failed test / an errorClass row in the ledger at nearby ts)
and a subsequent turn resolved it. Correlation key: `parentId` chain +
timestamps inside one session file; cross-check against ledger `ts` for the
serving lane. This is a *generator heuristic*, not a guarantee — every
emitted pair must still pass the eval gate (I-P3: provenance mandatory,
unprovenable rows dropped).

### F5 — engine serves adapters, not new models, per slot
In-tree engine (`engine/tools/server/server.cpp:284-285`): `GET/POST
/lora-adapters` set *global* adapter scale; per-request `lora:[{id,scale}]`
overrides it (README:606, `server-context.cpp:1966` `construct_lora_list`).
`--lora-init-without-apply` preloads at scale 0. **A slot serves ONE base
model** (`-m`); different slots serve the same model under the
`parallel=N` model (dual-slot trial 2026-11-21). Consequence:
- **S3 (no-training lane trial) cannot run a *different small model* on the
  live engine's slot 1** without a second engine instance.
- The clean local option: a **second engine instance** (own port, own
  `--alias`, small GGUF) — the box has 39 GiB DRAM and a spare 12 GB 3060;
  a Qwen-class 0.5–12B GGUF fits. `~/Models` already holds
  `qwen2.5-0.5b-instruct-q4_k_m.gguf` (469 MB), `gemma-3-1b-it-Q4_K_M.gguf`
  (769 MB), `gemma-4-12b-it-Q4_K_M.gguf` (6.7 GB).
- **S5 (adapter promotion) needs NO second engine**: load the adapter with
  `--lora` on the LIVE engine, flip scale via `POST /lora-adapters`, and the
  lane that routes to the local endpoint now serves base+adapter. Rollback
  = scale 0 / remove adapter file. Base GGUF untouched (I-P2).

### F6 — dream-eval can already score any endpoint
`dream-eval/index.ts` POSTs `POST {base}/completion` with a `model` field;
`scoreLanes` (S4, `d958d67`) walks arbitrary lanes. A second engine's
`--alias` (or the local lane id) is evaluable with zero changes.

## G — Proposed plan (rungs 2+3)

**D1 — dream generator (pure + extension).**
New `_shared/dream-generator.ts` (pure):
- `readTranscriptPairs(files, capBytes)` → extract (user-question,
  assistant-answer) pairs from pi session JSONLs, bounded by bytes (reuse
  sleep-lane's window discipline), each tagged with session file + ts
  (provenance).
- `dreamPrompt(pairs)` → I6-inherited quoted-DATA framing (injection
  resistant).
- `gatePairs(raw, evalHash)` → the gate: keep only pairs whose answer
  verifies under the SAME judge as dream-eval (numbers matched, coverage,
  no parrot) AND carry provenance. Version the corpus by content hash.
- Corpus → JSONL under `~/.vitriol/officina/dream/`, versioned.

**D2 — small-model lane trial (no training).**
- Bring up a second engine instance for a small GGUF (config in
  `profiles/`, own port, `--alias` = a caduceus lane id) OR simply register
  an existing small local endpoint as a lane.
- `/dream-eval --lanes` scores it vs the frozen eval (already implemented).
- Promotion = lane-registry rank change only when the eval report shows
  ≥ baseline on the protected classes (I-P1).

**D3 — invariants.**
- I-P3 (provenance) — every corpus row traceable to a session file + ts or
  MEMORY fact; untraceable dropped. Pinned by a provenance test.
- I-P5 (injection resistance) — corpus prompts inherit the sleep-lane
  quoted-DATA framing. Pinned by a prompt-shape test.
- I-E1 (frozen eval) — corpus hash separate from eval hash; the gate never
  mutates the eval set. Pinned by hash-stability test.
- I-P1 (measured promotion) — lane promotion only on a passing eval
  report. Pinned by the existing promotion-rule tests.
- I-P4 (reversible) — removing the small-model lane / adapter restores
  prior routing exactly. Pinned by lane-registry test.

## H — Open questions for the owner
1. Which small GGUF for the trial lane? (qwen2.5-0.5b is tiny/cheap;
   gemma-4-12b is stronger but ~6.7 GB on the spare 3060.)
2. Second engine instance (separate port, own unit) vs reusing slot 1 of
   the live dual-slot engine? The latter CANNOT serve a different model.
3. Should D1 (generator) ship first as its own commit (useful standalone:
   a curated practice corpus), with D2 (lane trial) as the next stage?
