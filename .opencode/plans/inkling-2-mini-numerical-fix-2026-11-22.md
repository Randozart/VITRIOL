# Inkling-2-Mini numerical correctness — fix the one-token latch (2026-11-22)

Status: CONFIRMED 2026-11-22 (owner: "confirm"). Follows
`inkling-2-mini-dual-gpu-bringup-2026-11-22.md` (S2: the model loads and decodes
across both GPUs — CUDA0 8711 MiB / CUDA1 7149 MiB, EXIT 0, 10.3 t/s gen /
24.5 t/s prompt at `-ub 32 -c 1024 -fa off -ts 27,21 --no-mmap`) and the
forensic RESEARCH pass (this file's §Research).

## Symptom (measured)

Greedy decode of a 20-token run latches to ONE repeated token, but the token
IS prompt-dependent:

- `The capital of France is` → `message message message …`
- `Water boils at a temperature of` → `temperature temperature …`

So the first step is roughly right and then the state degrades. Signature of
a recurrent/KV-state bug across decode steps, NOT a wiring or placement bug.

## Research findings (2026-11-22, forensics against upstream `8a97fa98`)

The port was diffed line-by-line against
`/scratch/briev-builds/llama-upstream` (upstream HEAD `8a97fa98`, the reference
arch). Result: the port is **remarkably clean** — every re-expression
argument-identical to upstream, EXCEPT one.

| item | vs upstream | verdict |
|---|---|---|
| `inkling2.cpp` (model, tensor load, MoE call, rel-bias `kq_b` construction, attention call) | verbatim (only `no_build{}` + `prec_policy` drops) | CLEAN |
| `inkling2-loop.cpp` | verbatim | CLEAN |
| `llm_graph_input_mem_hybrid_iswa_rel::set_input` / `can_reuse` / `build_inp_mem_hybrid_iswa_rel` | matches | CLEAN |
| `llama_kv_cache::set_input_relative_pos` | line-for-line identical | CLEAN |
| `llm_graph_context::build_conv_state_store` | verbatim identical (the recurrent state store) | CLEAN |
| `llm_graph_context::build_moe_ffn` fused `gate_up_exps` path + `LLM_FFN_SITU` | verbatim identical (formula matches) | CLEAN |
| inkling2 `build_moe_ffn` call args | argument-identical to upstream | CLEAN |
| **rel `build_attn` — where `kq_b` is applied** | **DIFFERENT — see §Stage 1** | **SUSPECT** |

### The one semantic difference (the suspect)

- **Upstream** (`llama-graph.cpp:4411`, rel `build_attn` overload):
  `kq_mask = kq_b + attn_mask` — the relative-position bias is **folded into
  the causal mask**; attention then runs with the combined mask and **no
  separate `kq_b`**.
- **Fork** (my thin forward at `llama-graph.cpp:3235` → `build_attn_mha`):
  `kq = kq + kq_b` on the raw scores, **separately** from `kq_mask`, then
  `ggml_soft_max_ext(ctx0, kq, kq_mask, …)`.

Both are *supposed* to be equivalent for `-fa off` (softmax is linear in the
additive term), but they are not the same expression. This is the one change
that could make step-1 attention roughly right and then degrade as the KV
cache grows — a perfect match for "one token, prompt-dependent, latches."

Everything else is ruled out by the forensic diff (model, MoE, SITU, rel-pos
bias, recurrent state store, layer placement, graph expansion, node budget).

## Plan

### Stage 1 — Prove/refute the rel-bias hypothesis (cheapest)

Rewrite the thin rel `build_attn` (`src/llama-graph.cpp:3235`) to fold the rel
bias **into the causal mask**, exactly as upstream:

```cpp
ggml_tensor * llm_graph_context::build_attn(
        llm_graph_input_mem_hybrid_iswa_rel * inp,
        ggml_tensor * wo, ggml_tensor * wo_b, ggml_tensor * wo_s,
        ggml_tensor * q_cur, ggml_tensor * k_cur, ggml_tensor * v_cur,
        ggml_tensor * kq_b, ggml_tensor * sinks,
            float     kq_scale, int       il) const {
    const bool is_swa = hparams.is_swa(il);
    // upstream structure: kq_mask = kq_b + attn_mask, then NO separate kq_b
    ggml_tensor * kq_mask = is_swa ? inp->get_attn()->get_kq_mask_swa()
                                   : inp->get_attn()->get_kq_mask();
    kq_mask = ggml_add(ctx0, kq_b, kq_mask);
    return build_attn(inp->get_attn(), wo, wo_b, wo_s, q_cur, k_cur, v_cur,
                      /*kq_b=*/nullptr, sinks, nullptr, kq_scale, il);
}
```

Rebuild `llama-cli` (target only). Gate run:
```
./build/bin/llama-cli -st -m ~/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf \
  -ngl 99 -ts 27,21 --no-mmap -ub 32 -c 1024 -fa off -n 40 --temp 0 \
  -p 'The capital of France is'
```
Decision:
- **Output changes (coherent or different)** → hypothesis confirmed; go to S3.
- **Output unchanged (still one token)** → hypothesis refuted; go to S2.

This is 1 build + 1–2 runs (~10–20 min). It is decisive and cheap either way.

### Stage 2 — (only if S1 refuted) isolate the recurrent state

If the rel-bias fold does nothing, the bug is the recurrent/shortconv state
across steps. Add a dev-only, stripped-in-final probe:
- `VITRIOL_INKLING_DBG_STATE=1` gated `fprintf` of `build_conv_state_store`'s
  `history`/`recr`/`s` contents at steps 1/2/3, and the shortconv `x` window.
- Cross-check the `convolve()` offsets (`2*n_embd_k_gqa`, `+n_embd`) against the
  upstream call (they are argument-identical, but *value*-check the offsets at
  runtime).
- Diff the shortconv state the model carries at step 3 against what it *should*
  be.

Rebuild, run `-n 3 --temp 0`, read the probe. Identify the exact state that
diverges.

### Stage 3 — Root-cause fix + regression test

Implement the actual fix (from S1's confirmed rel-bias fold, or from S2's
identified state bug).

Regression tests (both committed):
1. **Unit** `engine/tests/test-inkling2-attention.cpp`:
   feed a 3-token sequence, assert the attention output at step 3 uses the
   rel bias gathered from the **causal** structure (distance to each K/V cell),
   not raw scores. Negative control: assert the pre-fix structure produces a
   different output (so the test actually discriminates).
2. **CLI smoke** `engine/tests/inkling2-coherent-32tok.sh`:
   greedy `-n 32 --temp 0` decode; assert **≥ 2 distinct tokens** in the output
   (a one-token latch fails this).

### Stage 4 — Cleanup + commit

- Remove all `INKLING_DBG` / `VITRIOL_INKLING_DBG` instrumentation (final tree
  zero).
- Rebuild `llama-cli` clean.
- Commit per stage (S1, S2-if-run, S3, S4).
- Record evidence (before/after text, timings, tensor MiB) in this file.

## Invariants

- **I1 (parity):** every non-Inkling arch (incl. the live 27B) is
  byte-identical. The rel-bias fold lands only in the
  `llm_graph_input_mem_hybrid_iswa_rel` `build_attn` overload — the only caller
  is Inkling-2. (Pinned by `i1-no-engine-diff`: the 27B config
  `probe-recommend` + a 27B `-st` smoke before/after are identical; and the
  `test-inkling2-attention.cpp` unit only exercises the rel path.)
- **I2 (reversibility):** the live `vitriol-server.service` is restored and
  healthy (`/health {"status":"ok"}`) before ANY stage is called done.
  (Pinned by `i2-engine-restored`.)
- **I3 (no debug shipped):** the final tree contains zero
  `INKLING_DBG` / `VITRIOL_INKLING_DBG` instrumentation.
  (Pinned by `grep -r INKLING_DBG engine/src/` → empty.)
- **I4 (coherent gate):** Inkling-2-Mini greedy decode emits ≥ 2 distinct
  tokens over a 32-token run and is semantically responsive to the prompt.
  (Pinned by `inkling2-coherent-32tok.sh` + the `test-inkling2-attention.cpp`
  unit.)

## Gates / named checks

- `s1-rel-bias-fold`: the thin rel `build_attn` folds `kq_b` into `kq_mask` and
  passes `kq_b=nullptr`; run changes output vs baseline (or is refuted).
- `s2-state-probe`: (only if S1 refuted) the dev probe identifies the divergent
  recurrent state.
- `s3-fix`: the root-cause fix is in; `test-inkling2-attention.cpp` +
  `inkling2-coherent-32tok.sh` pass.
- `i1-no-engine-diff`, `i2-engine-restored`, `i3-no-debug`: invariants above.

## Cost / risk

- 2–6 engine builds (target `llama-cli` only), 10–20 min per `-fa off` run
  (ctx 1024, `-ub 32`). No training, no downloads.
- The 27B engine is stopped ONLY for the test windows and restarted via
  `systemctl start vitriol-server.service` between them (I2).
- Risk: S1 may do nothing (hypothesis wrong) — cheap and decisive; S2 then
  isolates the real state bug. S2's fix may be subtle (shortconv `history`
  view / `recr` layout) and might need one more iteration.

## Status

- 2026-11-22: CONFIRMED, writing plan. (owner "confirm")

## GRIND results (2026-11-22)

### S1 — rel-bias fold: REFUTED (as equivalence predicted)

Folded `kq_b` into the causal mask exactly as upstream
(`kq_mask = kq_b + attn_mask`, `kq_b=nullptr` into `build_attn_mha`). Output
**byte-identical** to before (`message` × 40, 13.0 t/s gen). Confirms the two
forms are equivalent under softmax — the hypothesis was wrong, and cheaply
disproved. The fold is kept because it matches upstream's expression.

### Ruled out by measurement

| hypothesis | test | verdict |
|---|---|---|
| rel-bias placement (raw scores vs mask) | S1 fold, run | **equivalent** — output unchanged |
| prompt format / chat template | extracted the GGUF template (`<\|message_user\|>`/`<\|message_model\|>`, `add_bos=false`, all special tokens present in vocab), ran a fully-templated greedy prompt | **still `message`** → not a formatting issue |
| fork's own phantom-slot placement | — | already fixed earlier (`n_layer_phys`) |
| upstream as a bisect reference | built upstream `8a97fa98` (`/scratch/briev-builds/upstream-ik-build`) + patched only the placement | **unusable**: stock upstream OOMs (15 861 MiB lump on CUDA0); patched upstream splits layers (28 on CUDA1) yet still allocates one 15 861 MiB lump on CUDA0 → upstream HEAD cannot run this model on 2×12 GB either |

### S2/F — REAL BUG FOUND AND FIXED: V was aliased to K

The fork's iswa attention overload
(`llama-graph.cpp:3238`, `build_attn(llm_graph_input_attn_k_iswa*)`) writes only K
to the KV cache (`cpy_k`) and then builds

```cpp
ggml_tensor * v = ggml_view_4d(ctx0, k, v_cur->ne[0], k->ne[1], k->ne[2], k->ne[3], ...);
```

i.e. **V is a view of K** — the MLA trick. That is correct for its only other
users, `deepseek4.cpp` and `dflash.cpp`, which are MLA models where V shares the
K cache. **Inkling-2 has its own V projection**, so V was garbage and attention
contributed nothing. Upstream instead does `cpy_k(k_cur)` **and** `cpy_v(v_cur)`,
then reads `k = get_k(il)` / `v = get_v(il)`.

This is exactly the observed symptom: the model echoes a word from the prompt
(`Water boils at a temperature of` → `temperature`) but never integrates context,
then latches on one token.

Fix (`dcc67dc`): the rel path now owns its KV store/read — `cpy_k` + `cpy_v`,
`get_k` + `get_v` — and does not delegate to the MLA overload.
Parity: `build_inp_mem_hybrid_iswa_rel` has exactly one user
(`models/inkling2.cpp`), so no other arch is touched (I1).

**Measured effect:** output changes — `message message …` → `0. 0. 0. …`.
V is now contributing, but the text is still degenerate, so at least one more
graph bug remains.

### Open leads (next session)

1. `hparams.is_mla()` is false for inkling2 (no MLA key/value lengths in the
   GGUF), so a real V cache exists (`llama-kv-cache.cpp:291 has_v = !is_mla`) —
   confirmed the fix has a valid target.
2. Remaining suspects, in order: the shortconv/recurrent state
   (`convolve()` + `build_conv_state_store` + the `history` view across steps);
   `v_trans` layout between `cpy_v` and `build_attn_mha`'s transpose branch;
   the SWA `v_idxs` path; `attn_res`/`mix()` block residuals.
3. Debug hygiene: the tree is clean of `INKLING_DBG`/`VITRIOL_INKLING_DBG`; no
   new instrumentation was added this session.

### Invariant status

- **I1 parity:** the change is confined to the rel overload (single user). The
  live 27B engine is unaffected; rebuilt and running.
- **I2 restored:** `vitriol-server.service` active, `/health` `{"status":"ok"}`,
  both GPUs loaded (11032 / 10090 MiB).
- **I3 no debug:** tree clean.
- **I4 coherent gate:** NOT MET. Still degenerate.

### Operational note (important)

`vitriol-server.service` has `Restart=always`. `kill -9` on an engine PID makes
systemd respawn it and it immediately re-takes both GPUs, which silently OOMs
every subsequent Inkling test. Always use `systemctl stop vitriol-server.service`
before a model-load window, and check `nvidia-smi --query-gpu=memory.free`
rather than only the used column.
