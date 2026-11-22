# Inkling-2-Mini: fix the shortconv recurrent state (2026-11-22)

Status: CONFIRMED 2026-11-22 (owner: "confirm").

Follows `inkling-2-mini-numerical-fix-2026-11-22.md` (V-aliased-to-K fixed,
output changed `message…` → `0. 0. 0. …`) and
`inkling-2-cross-engine-implementations-2026-11-22.md` (cross-engine research:
every engine caches convolved K/V and keeps a per-layer per-conv-site recurrent
state holding the last 4 raw pre-conv inputs).

## Problem

Greedy decode of Inkling-2-Mini on both GPUs is degenerate: it emits one token
repeatedly and never integrates context, though the token depends on the prompt
(`Water boils at a temperature of` → `temperature…`). Every model-side file and
every graph helper is verbatim against upstream `8a97fa98` and against the HF
reference. The one component not ported verbatim is the **fork's own recurrent
memory** (`llama_memory_recurrent`, `s_copy_main` / `s_copy_extra`), which the
rel attention path uses for the shortconv history.

Cross-engine research pins why this matters: the 4-tap causal shortconv is
applied to K and V (and the attention output and the MLP output) **before**
caching, and its input history lives in that recurrent state. If the history
never advances, each step's conv sees only the current token — attention then
has no local context, which is exactly the observed symptom.

## Stages

### S1 — Settle H1: does the conv history advance? (decisive, cheap)

Instrument (dev-gated, stripped in S5) the conv recurrent state for one site:

- in `models/inkling2.cpp::convolve()`, when `getenv("VITRIOL_INKLING_DBG_STATE")`
  is set and `offset == 0` and `il == 0`, `fprintf` the first few values of
  `history` (the state view) before the concat;
- and in `llm_graph_input_mem_hybrid_iswa_rel::set_input`, print
  `mctx->get_recr()->s_copy(i)` for `i < n_rs`.

Run `-n 3 --temp 0`, read the log. Decision:

- **history stays zero across steps** → H1 confirmed, go to S2.
- **history advances** → H1 refuted, go straight to S3 which becomes the main
  line of investigation.

### S2 — Fix the conv state read/write mismatch

Depends on what S1 shows. Two candidate repairs:

1. If the *write* works but the *read* is stale: align
   `llm_graph_context::build_rs(inp, s, state_size, n_seqs, get_state_rows)`
   with the fork's actual `build_conv_state_store` target cells — the fork
   passes `(inp->s_copy_main, inp->s_copy_extra)` where upstream passes
   `(state_copy, inp->s_copy_main)`. The rollback-ring s_copy protocol may need
   an explicit "single snapshot" path for `n_rs_seq == 0`.
2. If neither works: port upstream's `build_rs` + `llm_graph_input_rs` wiring for
   this arch, keeping the fork's rollback ring intact for the GDN models that
   depend on it.

Either way: no behaviour change for any arch other than inkling2.

### S3 — Companion checks (run alongside S2; each < 1 build)

- **H2** — assert `inp->get_v_idxs_swa()` is non-null and equals
  `get_k_idxs_swa()` at the first SWA layer (the SWA V path has never had a user
  besides inkling2).
- **H3** — one run with `-fa on` to exercise the other `v_trans` branch of
  `build_attn_mha`; degenerate *differently* vs *identically* is informative.
- **H4** — print `hparams.is_swa(il)` and `n_head_kv(il)` for `il` in 0..47 and
  diff against `layer_phys(il)` expectations (phantom-slot array fill).
- **H5** — assert `blocks.size() == il / hparams.attn_res_block_size` at the top
  of each `build_layer` (attn_res bookkeeping).

### S4 — Gate

```
llama-cli -st -m ~/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf \
  -ngl 99 -ts 27,21 --no-mmap -ub 32 -c 1024 -fa off -n 32 --temp 0 \
  -p 'The capital of France is'
```

Pass = ≥ 2 distinct tokens **and** a semantically plausible continuation
(not a prompt echo). Then repeat with the GGUF's own chat template
(`<|message_user|>` / `<|message_model|>` / `<|content_text|>`) and require the
same.

### S5 — Commit + restore

Commit per stage. Strip every `INKLING_DBG` / `VITRIOL_INKLING_DBG` probe.
`systemctl start vitriol-server.service`, assert `is-active` + `/health` ok.

## Invariants

- **I1 (parity):** no arch other than inkling2 changes behaviour. The rel
  attention path (`llm_graph_input_mem_hybrid_iswa_rel`) and the inkling2 model
  files have exactly one user; the fork's recurrent memory must keep its
  rollback-ring behaviour for the GDN/hybrid models that rely on it.
  *Pinned by:* `i1-parity` — before/after `llama-cli` on the live 27B GGUF
  produces the same first 16 greedy tokens, and `libvitriol probe-recommend`
  output for the 27B GGUF is unchanged.
- **I2 (restorability):** `vitriol-server.service` is active and healthy before
  any stage is called done. The unit has `Restart=always`, so it is stopped with
  `systemctl stop` (never `kill -9`) and restarted with `systemctl start`.
  *Pinned by:* `i2-restored` — `systemctl is-active` = active and
  `/health` returns `{"status":"ok"}`.
- **I3 (no debug shipped):** the final tree contains zero
  `INKLING_DBG` / `VITRIOL_INKLING_DBG` occurrences.
  *Pinned by:* `i3-clean` — `grep -rn "INKLING_DBG" engine/src/` is empty.
- **I4 (coherence gate):** greedy decode of a 32-token run emits ≥ 2 distinct
  tokens and the continuation is semantically plausible, both raw and templated.
  *Pinned by:* `s4-coherent-gate`.

## Named checks

| name | what it proves |
|---|---|
| `s1-state-advances` | the conv recurrent state advances across decode steps (or is proven stale) |
| `s2-conv-fix` | the state read/write agree; the conv sees real history |
| `s3-companions` | SWA V idxs non-null; `-fa on` behaves; phantom-slot hparams correct; attn_res blocks correct |
| `s4-coherent-gate` | coherent 32-token continuation, raw and templated |
| `i1-parity` | 27B unchanged |
| `i2-restored` | engine healthy |
| `i3-clean` | no debug left |

## Risks

- S1 may show the state *is* advancing, which moves the bug to H3/H4/H5. That is
  a useful outcome, not a failure — S3 already covers it.
- The fork's recurrent memory is shared with working models; a wrong fix could
  regress GDN/Qwen3Next. Mitigation: the fix must be arch-gated to inkling2, and
  I1 pins the 27B.
- Earlier sessions lost a lot of time to environment problems, not code:
  `vitriol-server.service` respawning on `kill -9` and re-taking both GPUs.
  Always `systemctl stop` first and check `memory.free`, not `memory.used`.

## Status

- 2026-11-22: CONFIRMED, starting S1.

## GRIND results (2026-11-22)

### S1 — conv recurrent state: H1 REFUTED (measured, not assumed)

Added a dev-gated probe at entry to `llama_context::decode` (with
`llama_synchronize` so the read is authoritative; `init_full()` verified
non-destructive — it only wraps `this`), dumping `get_r_l(0)`.

- Tensor: `cache_r_l0`, `ne = [19968, 1]`. **Size is exactly right**:
  `n_embd_r = n_sconv_channels·(l_cache−1) = 6656·3 = 19968`, and
  `6656 = 2·n_embd_k_gqa(1024) + n_embd(2304) + n_embd(2304)` — the four conv
  sites (K, V, attn-post, ffn-post) with 3 history columns each.
- The state is **zero for the first 3 decode calls and populated from call 3
  onward, changing every step** thereafter. (The leading zeros are the CLI's
  warmup decodes / the prefill boundary, not a stale state.)
- Therefore the shortconv history **does advance**. H1 is dead.

Also checked and cleared in the same pass:
- `n_rs_seq = 0` at runtime ⇒ `build_conv_state_store` takes the simple `cpy`
  path, not the rollback ring.
- `get_rs_z()` returns 0 only when `is_full`; the production path uses
  `init_update` (`rs_z = −1`), so `build_rs`'s `state_zero` view is zero-sized
  and cannot wipe the history it is about to read.

### S3 companions

- **H4 (phantom-slot hparams): PASSES.** Dumped `is_swa`/`n_head_kv`/`n_ff`/
  `n_expert_used`/`has_kv` for `il` 0..47. Slots 24..47 replicate the physical
  layers correctly — the full-attention (`is_swa=0`) positions are
  5, 11, 17, 23 and 29, 35, 41, 47, i.e. period 6 with the loop offset applied.
  `n_head_kv = 8`, `n_ff = 1152` throughout; no out-of-bounds or zeroed arrays.
- **H3 (`-fa on`): now works, after fixing a real bug.** The rel path folds the
  rel bias into the causal mask, which leaves the tensor F32; `ggml_flash_attn_ext`
  requires F16, so `-fa on` aborted at `ggml.c:5488`
  (`mask->type == GGML_TYPE_F16`). Upstream casts; the fold had dropped it.
  With the cast, `-fa on` **runs** — 46.4 t/s prompt / 14.1 t/s gen versus
  1.2 / 0.2 at `-fa off -ub 32`. The flash path had never been reachable for
  this arch. Committed.
- **H2 / H5** not reached this pass (see "next" below).

### NEW and most important finding: the logits are FROZEN across decode steps

Probed the logits buffer at each decode entry (200 058 vocabs):

```
decode 1  min=-5.5484 max=25.2588 mean=6.841782 argmax=13
decode 2  min=-6.4331 max=26.4842 mean=7.995384 argmax=220
decode 3  min=-6.4331 max=26.4842 mean=7.995384 argmax=220   <-- identical
decode 4  min=-6.4331 max=26.4842 mean=7.995384 argmax=220   <-- identical
decode 5  min=-7.2342 max=27.4734 mean=8.639627 argmax=220
```

**Three consecutive decodes produce byte-identical logit statistics and the same
argmax.** Appending tokens does not change the model's output, so greedy decode
necessarily repeats one token — which is exactly the observed symptom. The bug
is therefore not "the conv history is stale"; it is that **new context never
reaches the attention computation**.

Range is also a red flag: `max = 27.47` against
`final_logit_softcapping = 30` means the pre-softcap logit is ~75, i.e. the
residual stream is heavily saturated. A saturated stream plus frozen output is
consistent with attention contributing a constant.

### Where the next session must look

1. **Does the KV cache actually receive the new token?** Instrument `cpy_k` /
   `cpy_v` (the `k_idxs` / `v_idxs` values and the target cells) at decode 1 vs
   decode 2. If the write is a no-op, the cache never grows.
2. **Does the mask admit the past?** Dump a row of `self_kq_mask` — if all past
   cells are `-inf`, attention is a no-op and the output depends only on the
   current token (and after sampling a constant token, the logits freeze).
3. **Graph reuse:** `can_reuse_kq_mask` does check `ne[0] == n_kv`, and
   `set_inputs` is called on the reuse path (`llama-context.cpp:1431`), so reuse
   is *probably* fine — but with a unified cache `n_kv` is fixed for the window,
   so a stale-but-same-width mask would still pass the predicate. Worth pinning.
4. H2 (SWA `v_idxs` non-null) and H5 (`attn_res` block bookkeeping) remain
   untested.

### Invariant status

- **I1 parity:** ✅ `probe-recommend` for the 27B GGUF unchanged
  (`qwen35, 65 layers, 11.9 GiB`). Changes are confined to the rel path and the
  inkling2 probe (now removed).
- **I2 restored:** ✅ `vitriol-server.service` active, `/health`
  `{"status":"ok"}`, both GPUs loaded (11032 / 10090 MiB).
- **I3 clean:** ✅ `grep -rn "INKLING_DBG|VITRIOL_INKLING_DBG" engine/src/` is
  empty.
- **I4 coherent gate:** ❌ not met.

### Operational notes (cost this session real time)

- `vitriol-server.service` has `Restart=always`. `kill -9` on a GPU process
  makes systemd respawn the engine, which silently re-takes both cards and OOMs
  every subsequent Inkling run. Use `systemctl stop`, and read
  `nvidia-smi --query-gpu=memory.free` (not `memory.used`); a leftover
  `vitriol-server` from a `-st` run also holds both GPUs.
- `llama-cli -st` leaves its embedded `vitriol-server` alive after printing
  "Exiting…" — kill it by PID from `nvidia-smi --query-compute-apps` before the
  next run.

## CORRECTION + new evidence (2026-11-22, later pass)

### The "frozen logits" finding was a MEASUREMENT ARTIFACT — retracted

`process_ubatch` only copies logits into the host buffer when the sampler needs
raw logits; otherwise sampling happens on-device and `logits.data` keeps the
previous step's contents. Reading `logits.data` at decode entry therefore
compared stale buffers. **Do not chase this.**

### What the model actually does (measured, real output)

Sampling, `-fa on -ub 32 -c 1024 --temp 0.8 --top-p 0.95 --top-k 40 --seed 7`,
prompt `The capital of France is`:

```
0 . The capital of France is 0 0 .
### 0
### 0
```

Greedy, prompt `1 2 3 4 5`:

```
0 2 3 0 0 0 1 0
```

Three conclusions:

1. **Not stateless.** Output is varied under sampling and it *echoes the prompt*,
   so context does reach the model.
2. **Not dead.** `1 2 3 4 5` → `0 2 3 …` emits `2` and `3` in the correct
   positions — there is real structure, not noise.
3. **Badly corrupted numerics.** The dominant emissions are `0`, `###`,
   whitespace — filler tokens, i.e. a nearly-flat or badly-scaled distribution
   over filler.

So the remaining defect is a *numerical* corruption somewhere in the shared
graph machinery, not a missing state, not a frozen cache, not a prompt-format
issue.

### Everything checked and cleared in this pass

| candidate | evidence | verdict |
|---|---|---|
| conv recurrent state stale | `cache_r_l0 ne=[19968,1]` = exactly `6656·3`; advances every step after warmup | CLEARED |
| `n_rs_seq` rollback ring | 0 at runtime ⇒ simple `cpy` path; `rs_z=-1` so `build_rs` cannot zero the history | CLEARED |
| phantom-slot hparams | `is_swa` = 0 at 5,11,17,23,29,35,41,47 (period 6 + loop offset); `n_head_kv=8`, `n_ff=1152` for all 0..47 | CLEARED |
| KV slots for looped passes | cache sizes by `hparams.n_layer_all` (48) — `llama-kv-cache.cpp:162`; `has_kv=1` for all 48 | CLEARED |
| KV `share` callback clobbering loop passes | `llama_memory_hybrid_iswa` ctor has **no** `share` parameter | CLEARED |
| loop graph ctor | full semantic diff vs upstream: identical | CLEARED |
| flash-attention path | **was unreachable**; fixed (F16 mask cast). `-fa on` now works and is ~40× faster (46.4/14.1 t/s vs 1.2/0.2 at `-fa off -ub 32`) | FIXED |
| rel bias into mask vs scores | equivalent under softmax; reference says scores | CLEARED |
| chat template | templated greedy still degenerate | CLEARED |
| V aliased to K (MLA shortcut) | fixed `dcc67dc`; output changed | FIXED |

### The untested, highest-value experiment

**Run with most layers on the CPU** (`-ngl 10 …`). Every test so far has run
fully offloaded and split across two devices. Inkling-2's MoE is a fused
`ffn_gate_up_exps` expert tensor (17.6 GB IQ4_NL), and the fork has its own
`mul_mat_id` / expert-placement code paths (`vitriol-buffer.cpp`,
`build_lora_mm_id`) that no other arch in this fork exercises with this tensor
shape. If the CPU-only-ish run is coherent, the corruption is in the GPU
expert-matrix path or the layer split, not in the graph. If it is still
garbage, the corruption is in the graph math itself.

This is one run and it bisects the entire problem space in half.

### Invariants

- I1 parity ✅ (`probe-recommend` 27B unchanged: `qwen35, 65 layers, 11.9 GiB`)
- I2 restored ✅ (service active, `/health` ok)
- I3 clean ✅ (no instrumentation in tree)
- I4 coherent gate ❌

### Operational notes

- `vitriol-server.service` is `Restart=always`: `kill -9` makes systemd respawn
  it and it silently re-takes both GPUs. Use `systemctl stop`, then kill any
  leftover `vitriol-server` PID from `nvidia-smi --query-compute-apps`, and read
  `memory.free` (not `memory.used`).
- `llama-cli -st` leaves its embedded `vitriol-server` alive after "Exiting…".
