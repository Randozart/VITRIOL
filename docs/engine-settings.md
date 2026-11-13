# VITRIOL Engine Settings Codex

Every key in the live `~/.vitriol/config`, explained: what it does, the
current value, why it is set where it is (with links to the recorded
evidence), alternatives, and risks. Entry ids (`E-<section>.<key>`) are
stable anchors — probe reports and drift audits cite them.

Hardware context for every entry below: RTX 3060 12 GB (dev0, Ampere) +
RTX 3060 12 GB (dev1, Ampere), 39 GiB RAM with zram + disk swap, model
**Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K** (27B MoE, 65 layers,
~256 experts/layer, embedded MTP head), engine fork in-tree at
`engine/` (commit `30b12850`, upstream frozen).

Test discipline (applies to every entry): a setting is only "better" if a
measured run says so. Shallow benchmarks (short prompt, 64 decode tokens)
are cheap but **do not certify filled-context operation** — depth checks
at ≥13K filled tokens are the standard (see the 2026-08-24 lull
certification). Every benchmark RESULT must embed its full argv and
VITRIOL-FINGERPRINT line; flag drift is a review blocker. Heavy sweeps:
`libvitriol/sweep_controller.py`. Light probing: `vitriol probe …` (see
the plan `.opencode/plans/settings-codex-probe-2026-11-13.md`).

---

## [gpu]

### E-gpu.device — `0`
CUDA device index for the primary GPU. With `tensor_split` set, this is
the main device (logits/sampling). `0` = the 3060 that wears the KV cache
compute buffers. Alternative: none worth testing — the asymmetry below
already picks the split.

### E-gpu.rpc_servers — empty
Remote RPC layer-split targets (`host:port` list). Empty = local only.
The RPC devices land at the FRONT of the device list, so enabling requires
a 3-way `tensor_split` (e.g. `10,17,21`). Requires the engine built with
`-DGGML_RPC=ON`. Off: no remote GPU today. Revisit only when another box's
GPU should join the split (the probe subsystem's tensor-split candidates
would need an RPC-aware extension first).

### E-gpu.exclude_secondary — `false`
Excludes a secondary GPU entirely. Historical: the old GTX 960 (CC 5.2)
lacked kernel images and had to be excluded; both current cards are 3060s
(CC 8.6) so both participate. Change only if a foreign compute tenant
occupies dev1 (then prefer the fallback profile instead — see
E-gpu.tensor_split).

### E-gpu.tensor_split — `27,21`
Layer split across the two 3060s: 27/48 layers on dev0, 21/48 on dev1.
**Why**: the 2026-09-28 single-session blessing picked 27,21 at 262K
ctx with MTP, enabled by stopping `lares-vision.service` (~1.85 GiB dev0
freed). History: 24,12 (2026-08 era) → 27,9 → 27,21 → 24,24 (2026-09-28
fallback) → 27,21 (blessed). Alternatives: 24,24 fallback profile
`qwen38-buffedmod-208k` exists for foreign-tenant days (212K ctx, MTP 3).
Rollback: `vitriol config load qwen38-buffedmod-208k`.

### E-model.path — Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K
The daily-driver model: ISTA RCO quant (IQ3_S_XL) with `output.weight`
upgraded Q4_K→Q6_K (measured: −27.2% PPL error, −17.5% median KLD vs the
pre-upgrade build — see the 2026-09-28 single-session report). The
`-mtp` suffix means the GGUF embeds the MTP head. Alternatives on disk:
the older ISTA `Qwen3.8-27B-GSQ-RCO-IQ3_S-mtp.gguf` (rollback target).

### E-model.context — `262144`
Native context window, no RoPE scaling. The 09-28 blessing certified
262K + MTP n=2 fits only with lares off and ts 27,21. Deeper = more KV
(≈19.1 KiB/token at q4_0/q4_0 ≈ 4.7 GiB at 262K) but depth, not window,
is the wall — see E-kv.mode.

### E-model.threads — `4`
CPU threads for prompt processing. Host has enough cores for 4 prompt
threads; the GPUs do the heavy lifting. Not benchmarked recently — a
`probe load` candidate (threads 2/4/8 sweep is cheap on the synthetic
model).

### E-model.ngl — `99`
All layers offloaded. Non-negotiable for this class: partial offload
pours expert traffic over PCIe. Keep at 99.

### E-model.expert_count — `0`
Expert-count override (0 = model default = 256 used per tensor as
authored). Overriding truncates experts and corrupts quality for this
model family — leave 0 unless a probe-run documents otherwise.

### E-model.alias — `Lapis Occultus` (LOAD-BEARING)
Name reported via `/v1/models`. pi's model-sync identity matches on it.
Changing it silently breaks model selection in the TUI — change together
with `~/.config/opencode/opencode.jsonc` entries or not at all.

### E-model.mmproj — empty (LOAD-BEARING)
Vision projector. Empty = text-only, which is REQUIRED for slot
save/restore: multimodal engines gate the persistence chain (501) —
re-enabling vision silently disables warm resume. Re-enable only with
eyes open.

### E-model.n_cpu_moe — `0`
CPU-MoE (first N layers' experts in host RAM). Off: with both 3060s
resident, host RAM round-trips cost more than they save. Revisit only if
a third GPU or DDR5 arrives.

### E-model.mmap — `off`
Weights hot-loaded, not mmapped. `off` pins the weights in VRAM (they
fit) and avoids page-eviction stalls mid-decode. `on` would let the OS
evict cold pages — pointless when everything is resident.

## [vitriol]

### E-vitriol.mode — `off`
Streaming/DMA mode: `off` = weights fully VRAM-resident. The 2026-08-24
residency rule: stream only when weights exceed combined VRAM (35B-class
experiments); streaming a fitting model starves GPUs on DDR3/PCIe expert
fetches. The 27B IQ3_S_XL fits both 3060s → `off` is correct. Change only
for a bigger model.

### E-vitriol.disk_offload / store / store_path / store_source / max_locked_mb / lru_mb / lru_mb_gpu — all off/0/empty
The raw-zone expert store + LRU pinning family: only meaningful with
`mode != off` (stream/sync/async). All dormant at `mode = off`. The
2026-10-10 governor's RELIEVE ladder interacts with `store_path` when
enabled — keep dormant unless the residency rule flips.

### E-vitriol.store — `0`
Raw-zone expert store enable (pairs with `mode=stream` +
`max_locked_mb>0`). Dormant with `mode = off` — see E-vitriol.mode.

### E-vitriol.store_path — empty
Store device/file path. Empty = disabled (see E-vitriol.store).

### E-vitriol.store_source — `store`
Where the store fills from: `store` | `gguf`. Only meaningful when the
store is enabled (E-vitriol.store).

### E-vitriol.max_locked_mb — `0`
Lazy per-expert host-pin budget in MB (0 = pin everything when streaming;
needs RAM ≥ experts). Dormant with `mode = off`.

### E-vitriol.lru_mb — `0` and E-vitriol.lru_mb_gpu — empty
LRU VRAM cache for expert tensors in MB (global; `lru_mb_gpu` is the
per-device CSV override). Only effective with FP16/FP32 experts and
`mode != off` — IQ3_S quantized experts are excluded by design. Dormant.

### E-vitriol.verbose — `false`
Verbose engine logging. Off: log volume is large; the journal carries the
essentials (fingerprint, layer offload map).

### E-vitriol.output_cache — `off`
Approximate expert-output reuse (previous token's experts). Off: exact
compute. Historical experiments showed quality risk without reliable
speedup on this model; pin_first_n_layers auto-disables it anyway.

### E-vitriol.predictive_prefetch — `off`
Cross-layer + temporal expert prefetch (DMA-stream overlap). Dormant with
mode off (same reason as the store family).

### E-vitriol.pin_first_n_layers — `0`
Pin the first N layers' experts in VRAM (~220 MB/layer, targets the fast
MMVQ/MMQ kernels). Off here: benefits showed on other quant families;
with everything resident the win is unproven on BuffedMod. **Experiment
candidate** (probe protocol: pin 0/4/8 sweep on synthetic + one real
depth check), interacts with KV offload for N>10.

### E-vitriol.prune_experts — `0`
Drop bottom N of 8 active experts before compute. Experimental, quality
risk (creative tasks especially). Off until a measured quality gate
(perplexity/KLD) says otherwise.

### E-vitriol.reasoning — `off`
Thinking-mode disable (required for the Qwen3.6 family lineage).
Keep off unless the model card changes.

## [server]

### E-server.host — `127.0.0.1`
Loopback bind. Never expose: the API has no auth layer of its own.

### E-server.port — `8279`
API port. The TUI, opencode entries (`qwen38-mtp`/`qwen38-262k`),
Hermes config all target 8279. Do not change casually — load-bearing
across configs.

### E-server.parallel — `1`
One slot. The single-session doctrine (2026-09-28): parallel slots split
KV and break the warm-resume economics. Multi-session concurrency is
handled above the engine (overflow lanes), not here.

### E-server.slot_context — empty
Per-slot context override. Empty = `model.context` for the single slot.

### E-server.slot_save_path — `/scratch/kvtier`
KV slot mirror on the XFS scratch NVMe (63→126 G free). The boot sidecar
`restore-kv-slot` warms `/scratch/kvtier/slot0.bin` at boot. XFS is
deliberate: no COW penalty, NVMe-fast. Keep.

### E-server.cache_ram — `8192`
Host-RAM prompt cache (MiB) for spilled slots. 2026-09-27 raised 2048→8192
(measured slot states 2.5–7 GiB) so same-context next turns re-prefill
delta-only (certified 745/755 cached, 0.5 s). **0 is forbidden** (no
readiness). The trim history: 256 was the OOM-era value; 8192 is the
current blessed point.

### E-server.cache_idle_slots — `off`
Idle-slot KV→host backup. Off per the 2026-09-04 host-memory trim: the
backup was a swappable reservoir inflating engine OOM badness.

### E-server.cache_reuse — `0`
KV-shift salvage after prompt-head divergence (min chunk; rebis uses 256).
Off here: the shift path saved mode-switch/cancel turns on other builds;
on the current single-slot daily driver those turns are rare — **experiment
candidate** (protocol: force a mode-switch turn, measure re-prefill tokens
with reuse 0 vs 256).

### E-server.ctx_resize — `1`
Live KV-window swap (`POST /ctx/resize`), gated by this env/config key;
fingerprint `cxr=`. The governor's RESIZE rung uses it. Keep on while the
governor is deployed.

### E-server.chat_template — `~/.vitriol/chat-template-tolerant.j2`
Custom Jinja template, "tolerant" variant: permissive enough for the
Qwen3.8 hybrid chat format without the strict-template failures. Changing
it changes every prompt's serialization — treat as load-bearing.

## [memory]

### E-memory.mode — `off`
Emulated-memory mode. Off (never adopted on this host).

### E-memory.semantic_mode — `off`
Semantic search over memories; requires sentence-transformers on the
host. Off — the workflow never adopted it; revisit only if a RAG-style
memory lane is wanted.

## [kv]

### E-kv.mode — `sparse`
Sparse-KV eviction (context lifecycle, 2026-09-03): cache beyond the
attention horizon is evicted by probe score instead of being pinned. This
is the daily driver's depth enabler. Alternatives: `standard` (everything
pinned — OOM at depth), `offload` (host-side — the trimmed reservoir).
Keep sparse.

### E-kv.score — `probe`, E-kv.score_every — `16`
Attention-probe scoring feeds eviction; cadence 16 decode steps. Cadence 4
was owner-tested and **rejected** (ejected too hard). 16 is blessed.

### E-kv.score_min_fill — `0.5`
Score only when cache fill ≥ 50% (2026-09-22): short agent turns skip the
graph-rebuild + stream-sync overhead (~18% shallow-decode penalty
removed). Blessed; keep.

### E-kv.quant_mode — `q4_0`
K cache at q4_0 (~4 bits). At 262K the KV is ≈4.7 GiB total. History:
tq3_0 (3.5 bpw TurboQuant) was the depth-cert point on the merged base
(54,692 tok @ 9.21 t/s); q4_0 was the 09-28 blessing for BuffedMod at
262K + MTP. **V-quant warning applies**: q4_0 on V caused garbage on some
families — on THIS model the 09-28 certification ran q4_0/q4_0 and passed.
Alternatives: tq3_0/tq3_1s for more headroom (test protocol: depth cert
+ KLD check). V cache quant warning (E-kv.quant_mode_v): q4_0 on V caused
garbage output on some families — on THIS model the 09-28 certification
ran q4_0/q4_0 and passed; if re-certifying V-quant, run the KLD gate.

### E-kv.frozen_prompt — `off`
Keep the system prompt static to avoid re-prefixing. Off: the TUI's
preservation layer (scratchpad/task tails re-injection) mutates the
system prompt each turn anyway; freezing would fight it.

### E-kv.ubatch_size — `128` (E1 MEASURED 2026-11-13: keep)
Physical batch for prompt processing; lower = smaller compute buffers
(VRAM). **DRIFT FLAG**: the AGENTS working-config line says `-ub 64`
(blessed 2026-09-04 depth cert); the live file says 128. The 09-28
retarget history is unclear on which ubatch shipped. **Test protocol**
(E1): shallow 3×64 bench at ub 64 vs 128 on the synthetic grid, then one
13K-depth check per arm. Rollback: set 64.

### E-kv.checkpoint_every_n_tokens — `8192`
Slot checkpoint cadence (lull engine flag). 8192 balances IO vs
crash-loss granularity. Keep.

### E-kv.ctx_checkpoints — `4`
RAM checkpoint-ring depth per slot. 4 per the 2026-09-04 trim (32 was
multi-GB). **0 is forbidden** (heap corruption). Keep 4; `ckpts=` rides
the fingerprint.

## [engine]

### E-engine.mode — `vitriol-dma`
Execution mode (vitriol-dma | native). vitriol-dma is the fork's
DMA-capable path — required infrastructure for the streaming/store family
even while dormant. Keep.

## [lookup]

### E-lookup.tokens — `0`
Prompt-lookup decoding (draft from n-gram overlap). Off: MTP (below) is
the blessed draft source; PLD on top would fight it. Alternative for
models WITHOUT an MTP head.

## [spec]

### E-spec.type — `mtp`, E-spec.draft_n_max — `2`
Speculative decoding via the embedded MTP head, 2 draft tokens. The
2026-11-11 A/B at ~70K filled depth: acceptance 0.953 (n=2) vs 0.735
(n=3) — the 3rd token is net negative on BuffedMod-Q6K. History: n=3 was
the 09-28 point (+17% at 42.7K vs n=1); build/model-sensitive, re-certify
after any model or MTP-engine change. n=4 loads but is not faster.
Rollback: n=1 or 3.

### E-spec.adaptive_k — `0` (E2 MEASURED 2026-11-13: keep)
Acceptance-driven draft-length trim (shrink K when per-position acceptance
drops; full-K re-exploration every 128 drafts; fingerprint `specak=1`).
0 = fixed K. **Never A/B'd on BuffedMod** — experiment candidate E2:
adaptive_k 0 vs 1 at 42K and 130K depth, comparing t/s AND acceptance
stability. Risk: exploration overhead on already-stable acceptance.

### E-spec.draft_model / draft_device / draft_ngl — empty / empty / all
Separate draft model/device overrides. Empty = embedded MTP head on the
main devices (the 09-28 asymmetric split already gives dev1 the draft
context). `draft_type_k/v` empty = f16 draft KV — **q4_0 draft KV breaks
flash-attn for the MTP layer** (pp buffers ~1.2 GiB → load OOM); do not
quantize the draft KV on this model.

### E-spec.draft_device — empty
Dedicated device for the draft model. Empty = main devices (the 09-28
asymmetric split already parks the MTP draft context on dev1, ~3 GiB
headroom there). Set only if dev0 pressure returns.

### E-spec.draft_ngl — `all`
Draft-model layers in VRAM: all | auto | N. `all` with the embedded head
is correct — a partial draft offload would put the hottest small tensors
on PCIe.

### E-spec.draft_type_k — empty
Draft K-cache quantization override (f16 engine default). See the
draft-KV warning above: keep f16 on this model; q4_0 draft KV is a
certified OOM.

### E-spec.draft_type_v — empty
Draft V-cache quantization override. Same warning, same answer: f16. See the draft-KV
warning above: keep f16 on this model; q4_0 draft KV is a certified OOM.

## [chimera]

### E-chimera.mode — `auto`
Backend routing: auto → Vulkan path if available else CUDA-only. The
Vulkan chimera (all tensors via VK_EXT_external_memory_host) is an
alternative expert-streaming substrate; with `vitriol.mode = off` both
collapse to CUDA-only. Keep auto; revisit only with the streaming family.

## Drift audit (live vs blessed) — see stage 5 for full protocols

| Entry | Live | Blessed/reference | Status |
|---|---|---|---|
| E-kv.ubatch_size | 128 | 64 (2026-09-04 depth cert) | **RESOLVED — keep 128 (E1 below)** |
| E-spec.adaptive_k | 0 | 0 kept (E2 measured 2026-11-13: +1.0% at 42K, under the 5% bar — n_max=2 leaves no trim room) | **RESOLVED — keep 0** |
| E-server.cache_reuse | 0 | 256 on rebis builds | E3 protocol (rare turns) |
| E-vitriol.pin_first_n_layers | 0 | untested on BuffedMod | E4 protocol |
| E-kv.quant (K/V) | q4_0/q4_0 | tq3_0 on merged base; q4_0 blessed on BuffedMod | current ✓ |
| E-spec.draft_n_max | 2 | 2 (2026-11-11 A/B) | current ✓ |
| E-gpu.tensor_split | 27,21 | 27,21 (2026-09-28, lares off) | current ✓ |

## Experiment protocols (drift audit, 2026-11-13)

Each protocol: hypothesis · exact commands · pass/fail bar · rollback.
Fingerprint discipline applies to every run (RESULT embeds argv; check the
`VITRIOL-FINGERPRINT:` line — `spec=`, `ts=`, `cram=`, `ckpts=`, `cxr=`
must match the arm you think you are running). Probe reports land in
`~/.vitriol/probes/`.

### E1 — ubatch 128 vs 64 (live drift candidate)
Hypothesis: the blessed depth point was `-ub 64` (2026-09-04 cert); the
live file carries 128. On the 3060 pair the difference should be small
but measurable at depth.
- Shallow: `vitriol probe load --ubatch 64,128` (load-light models —
  ubatch curve only; expect pp to scale, tg roughly flat).
- Depth: for each arm, restart via the unit with an edited profile and run
  the 13K-depth check (3×64 decode at ≥13K filled) exactly as the 09-04
  cert did; record t/s + filled tokens.
- Pass/fail: ≥3% decode delta at depth in favour of one arm.
- Rollback: the live profile is untouched; profiles exist for every
  blessed point (`qwen38-buffedmod-208k` if external GPU tenants return).

### E2 — adaptive_k (never A/B'd on BuffedMod)
Hypothesis: acceptance-driven draft-length trim helps when acceptance
decays at depth; hurts when acceptance is stable (11-11 A/B: 0.953 at
n=2).
- A/B at 42K and 130K filled depth: `[spec] adaptive_k = 0` vs `1`
  (engine flag rides the fingerprint as `specak=1`).
- Pass/fail: ≥5% decode delta at 130K with no acceptance-rate collapse
  (log the per-position acceptance from the engine log).
- Rollback: set `adaptive_k = 0`.

### E3 — cache_reuse (rare-turn salvage)
Hypothesis: the KV-shift salvage saves re-prefill on prompt-head
divergence (mode switches, cancels).
- Force one mode-switch turn and one cancel-after-long-prefill turn per
  arm (`cache_reuse = 0` vs `256`); read the re-prefill token count from
  the server log.
- Pass/fail: ≥30% fewer re-prefilled tokens on the divergent turn with
  identical output quality (spot-check the reply).
- Rollback: `cache_reuse = 0`.

### E4 — pin_first_n_layers (untested on BuffedMod)
Hypothesis: expert pinning targets the fast MMVQ/MMQ path but the
weights are already VRAM-resident at `vitriol.mode = off`; the win is
unproven on IQ3_S_XL.
- Sweep `pin_first_n_layers = 0,4,8` at shallow depth only (pin 8 costs
  ~1.8 GiB VRAM — watch fit).
- Pass/fail: ≥3% decode delta at pin 4 — anything else is noise.
- Rollback: `0`.

### Probe subsystem cheat-sheet
```
vitriol probe hardware                  # GPUs, RAM/zram/swap, store read BW
vitriol probe load --ubatch 64,128,256  # ubatch throughput grid (load-light)
vitriol probe recommend                 # derived-KV ladder + ranked config
vitriol probe recommend --write-profile NAME   # opt-in profile file
vitriol probe synth --model PATH --scale tiny|mid --out FILE  # shape-mirror GGUF
```
Labels: MEASURED numbers come from llama-bench runs; EXTRAPOLATED numbers
come from the estimator + derived KV. Never mix them in a decision
without saying which is which.

## E1 result — ubatch 64 vs 128 (measured 2026-11-13)

Harness: `scripts/depth_cert.py` (repaired: engine binary path, `--spec-n`
passthrough), isolated server on :8299, live GGUF, `--window 262144
--ts 27,21 --kv q4_0/q4_0 --cache-ram 8192 --ckpts 4 --mode off
--spec mtp:n2`. Arms differ ONLY in `ub=` (fingerprint-verified). Decode =
3x64 greedy at depth; `filled` = tokens_evaluated.

| arm | ub | filled | decode t/s (mean of 3) |
|---|---|---|---|
| shallow | 128 | 290 | 23.33 |
| shallow | 64 | 290 | 23.48 |
| 13K | 128 | 13,075 | 22.77 |
| 13K | 64 | 13,075 | 22.56 |
| 42K | 128 | 42,175 | 21.27 |
| 42K | 64 | 42,175 | 20.23 |

Deltas (128 vs 64): shallow -0.6% · 13K +0.9% · **42K +5.1% (128
faster)**. Paired per-round at 42K, 128 won every round (+2.1, +0.8,
+0.2 t/s).

**Verdict**: keep ubatch **128**. The "blessed ub64" line predates this
build; ub64 regresses at the depth the operator actually works at. The
live config already carries 128 — **no change needed**. Caveat: sparse
KV/probe scoring were configured via env but inactive at these depths
(no eviction below the 50% min-fill); both arms share that condition.

## E2 result — adaptive_k 0 vs 1 (measured 2026-11-13)

Harness: depth_cert (isolated :8299), ub 128, spec mtp:n2, 42K filled,
identical fingerprints except `specak=`. Paired arms, 3x64 decode.

| arm | adaptive_k | decode t/s mean (3 rounds) |
|---|---|---|
| ak0 | 0 | 22.30 |
| ak1 | 1 | 22.52 |

Delta: **+0.99% for ak1** — under the 5% adoption bar, within round
variance (±2 t/s spread). Draft acceptance was IDENTICAL across arms
(0.89 → 0.62 → 0.48 declining, deterministic stream) — the trim changes
speculation cost, not generation. Root observation: with draft_n_max = 2
the trim ceiling is a single token — nothing meaningful to trim. adaptive_k
becomes interesting only at n_max >= 4, which this model rejects (n=4
loads but is not faster; n=3 acceptance collapses per the 11-11 A/B).

**Verdict**: keep `adaptive_k = 0`. Revisit only if draft_n_max ever
exceeds 2.
