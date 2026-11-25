# GDN raw gates — research for the reconciliation stage (2026-11-26)

**Status:** research complete; plan in `gdn-raw-gate-reconciliation-2026-11-26.md`.
**Question:** the raw-gate port engages and produces garbage. Where is the defect?

Reference tree: `~/kintsugi/engine-src/kintsugi-engine-8f12baacf/` (MIT).
Ours: `engine/` at `3355896` (GDN port reverted).

## 1. The graph half is textually identical to theirs

Line-by-line comparison of the two files that participate:

| site | theirs | ours (as ported) | verdict |
|---|---|---|---|
| `qwen35.cpp` beta | `beta_raw = beta` before `ggml_sigmoid` (:517-521) | same | identical |
| `qwen35.cpp` raw publish | `gdn_raw_alpha = reshape_4d(alpha, 1, H, T, S)` etc. (:532-542) | same | identical |
| `delta-net-base.cpp` K=1 | `raw = gdn_raw_beta && gdn_raw_alpha && gdn_raw_dt_bias && gdn_raw_a`; op gets `raw ? gdn_raw_alpha : g`, `raw ? gdn_raw_beta : b`, then `set_raw_gates` (:404-409) | same | identical |
| `ggml_gated_delta_net` construction | `op_params[0]=K` only; `ggml_new_tensor` zeroes the rest (:6560-6600) | same | identical |

So the four outputs hand to the op are the same four tensors in the same order from the same
projections.

## 2. The kernel algebra is identical too

Their activation, verbatim (`gated_delta_net.cu:36-46`):

```cuda
const float x = g[i] + raw_dt_bias[h];
g_out[i]    = raw_a[h] * ((x > 20.0f) ? x : logf(1.0f + expf(x)));
beta_out[i] = 1.0f / (1.0f + expf(-beta[i]));
```

and in the per-token kernel (`:196-201`, `:213-219`) the same two formulas, then `expf(g0)`.
Our activated graph computes `a * softplus(alpha + dt)` and `sigmoid(beta)` and hands those in; the
op then applies `expf`. Same function, different place.

## 3. Every raw-path precondition is asserted, and the asserts are compiled in

`ggml_gated_delta_net_set_raw_gates` (their `ggml.c:6592-6610`) asserts `op == GATED_DELTA_NET`,
`src[3]->ne[0] == 1` (scalar gate, not KDA), and `dt_bias`/`a` contiguous F32 with `nelements == H`.
The kernel asserts `!(raw && kda)` (`:556`). None fired — a violated precondition would have aborted,
not corrupted.

## 4. Hypotheses tested and eliminated

- **Snapshot slots (K > 1).** Their kernel wants `keep_rs == false` for raw. The failing run had **no
  drafter** ⇒ `cparams.n_rs_seq = 0` ⇒ `K = 1`. Eliminated.
- **Token loss in the non-MMA fallback.** `n_tail = K > 1 ? gdn_mma_tail(K) : (batch_invariant && n_seqs==1 ? gdn_mma_tail(K) : 0)`
  and `n_mma = n_tokens - n_tail`, but the final `GDN_LAUNCH` covers the **full** `n_tokens`
  (`:732-745`); the tail split only trims the MMA branch, which `mma_dev` disables on sm_86
  (`:645-651`). Eliminated by reading.
- **Garbage `op_params[2]` (`n_pend`, the replay selector).** Both trees set only `op_params[0]` and
  both allocate via `ggml_new_tensor`, which zeroes `op_params`. Eliminated.
- **A different CPU-side implementation.** Their CPU GDN *does* implement raw
  (`ggml-cpu/ops.cpp:10976-10981`, `:11081`); ours does not, but the failing run was fully
  offloaded, so the CPU path was never executed.

## 5. The finding that matters for the port's shape

**Our fork has a chunked GDN kernel that theirs does not.** `gdn_scan_enabled()` /
`gdn_scan_supported()` (`gated_delta_net.cu:480-492`) dispatch `n_tokens ∈ {2,3,4,6,8}` to
`launch_gated_delta_net_chunk` (`:636-650`) — the VITRIOL path that breaks the serial recurrence for
MTP verify batches. `grep scan` in their file returns **nothing**; their equivalent is the chunked
MMA kernel, which is Blackwell-only (dead here).

Consequence: the previous round imported their file *wholesale*, so it silently deleted our chunk
kernel and replaced it with a path that cannot run on sm_86. **That alone may account for the
measured slowdown (7.76-7.78 vs 8.07-8.80)**, independent of raw gates.

**Therefore the port must be raw INTO our file, not a file replacement.** The chunk kernel needs raw
support of its own — and their own MMA branch shows exactly how to give it without touching the
recurrence (`:688-697`: activate raw gates into temporaries with `gdn_activate_raw_gates` before the
chunk kernel runs). That is the pattern to reuse, not to re-derive.

## 6. Conclusion

The static comparison is exhausted: graph, construction, algebra and preconditions all match, and the
four hypotheses that could explain a *gross* failure by reading are eliminated. What remains is a
values-level difference that must be **measured** — either in the op (raw vs activated on identical
inputs) or, if the op is correct, in the graph (where the two paths first diverge).

The next stage is therefore instrumentation-first, not edit-first. Two isolations, in order:

1. **Op-level**: same tensors through `GGML_OP_GATED_DELTA_NET` twice — activated gates vs raw gates
   plus `dt_bias`/`a` — require agreement. Tells us whether the kernel's raw path matches the algebra
   it claims to replace.
2. **Graph-level**: only if (1) passes. Run the model with `GGML_GDN_RAW_GATES_DISABLE=1` and `=0` and
   diff the GDN output tensors to find the first diverging layer, which names the tensor that is wrong.

This is precisely the step the previous two rounds skipped, and the reason both ended in a revert.
