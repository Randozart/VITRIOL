# Incident: `unordered_map::at` in `pre_decode()` — probe capture leak

- **Date:** 2026-09-28 16:34
- **Severity:** engine-wide slot abort loop (every request to the affected slot
  failed until restart)
- **Component:** VITRIOL attention probe depth-gate (2026-09-22, inner
  `229238384`) interacting with MTP dual-context decode
- **Status:** fixed, rebuilt, restart pending

## Symptom

`/tmp/opencode/vitriol_gen.log` showed, starting ~4h into uptime, bursts of:

```
E srv  update_slots: pre_decode() failed: unordered_map::at
E srv    send_error: task id = 42203, error: pre_decode() failed: unordered_map::at
```

Each burst began once the scored KV cache filled past 50% and then every
subsequent task on that slot failed within milliseconds. Live config:
Qwen3.8-27B-GSQ-RCO + `--spec-type mtp`, `--cache-type-k/v q4_0`,
`VITRIOL_KV_SCORE=probe` (`score=probe` in the launch fingerprint).

## Root cause

The depth-gate introduced in `229238384` ("vitriol: depth-gate the attention
probe") lets the **local** gate decision diverge from the **global** probe
armed flag.

`llama_context::process_ubatch()`:

```
bool vitriol_probe_armed = vitriol_probe::active();   // global g_armed
if (vitriol_probe_armed) {
    auto * kv_ctx = dynamic_cast<llama_kv_cache_context *>(mctx);
    if (!kv_ctx || kv_ctx->fill_ratio() < vitriol_probe::min_fill()) {
        vitriol_probe_armed = false;                  // LOCAL false...
    }
}
...
gf = model.build_graph(gparams);                      // build_attn checks global active()
if (vitriol_probe_armed) {
    vitriol_probe::append_scores(res->get_ctx(), gf); // ...so this is SKIPPED
}
```

- `begin_batch()` sets the global `g_armed=true` at decode start
  (`llama-context.cpp:1678`).
- `build_attn()` pushes captures on the **global** flag:
  `if (vitriol_probe::active()) vitriol_probe::push_capture(il, q, kq_scale);`
  (`llama-graph.cpp:2892`).
- `append_scores()` is the only place that clears `g_captures`
  (`vitriol-kv-probe.cpp:118-131`).

When the fill gate set `vitriol_probe_armed=false` but the global flag stayed
true, the build still pushed captures yet `append_scores()` never ran. The
orphan captures survived into the next graph.

The next graph is often the **MTP draft context** (`common_speculative_draft`
called from `pre_decode` -> `llama_decode(ctx_dft)`), whose KV cache holds a
disjoint layer set (`filter: il >= n_layer`, `llama-model.cpp:2709`). When the
gate later passed, `append_scores()` iterated the stale captures and called
`src->get_k(ctx, cap.il)` (`vitriol-kv-probe.cpp:140`) for a layer absent from
that cache -> `llama-kv-cache.cpp:1369 map_layer_ids.at(il)` ->
`std::unordered_map::at`.

Deeper KV (>50% fill) is required for the gate to engage at all, which is why
the failure only appeared hours in, once a long agent turn pushed the cache
past `VITRIOL_KV_SCORE_MIN_FILL=0.5`.

## Fix

Ownership of the gate moved to the probe module so the build side and the
completion side cannot disagree, and captures are self-describing so a foreign
layer index can never be resolved:

1. **Per-capture source.** `capture_rec` now stores the owning
   `llama_kv_cache_context *`, and `push_capture(src, il, q, kq_scale)` takes it
   directly. The global `g_source` "last seen" is gone. `append_scores()`
   resolves each capture against `cap.src` (and drops the batch if captures mix
   sources). Because `build_attn` already calls `mctx_cur->get_k(ctx0, il)`
   before recording, any `(src, il)` pair is valid for that src by
   construction - the `map_layer_ids.at` miss is structurally impossible.
2. `vitriol_probe::cancel_batch()` (new): disarms the probe and drops the
   capture buffer.
3. `process_ubatch()` calls `cancel_batch()` the moment the fill gate overrides
   an armed probe, **before** `build_graph()`, so `active()` is false and
   `build_attn()` collects nothing.
4. `begin_batch()` clears stale captures at decode start (defense in depth; a
   decode must never inherit captures from a prior graph).
5. `push_capture()` ignores null `src`/`q`; dead `has_pending_output()` removed
   (it had no declaration and warned `-Wmissing-declarations`).

Files:
- `llama.cpp/src/vitriol-kv-probe.h`
- `llama.cpp/src/vitriol-kv-probe.cpp`
- `llama.cpp/src/llama-graph.cpp`
- `llama.cpp/src/llama-context.cpp`

## Build plumbing hardening

The CUDA link needs the Rust static lib. The old code read
`$ENV{VITRIOL_RUST_LIB}` inline at configure time, so any later reconfigure
without the env var silently dropped the link and broke `llama-server` with
undefined `vs_*` (this happened while building the fix). Replaced with a cached
`VITRIOL_RUST_LIB` CMake variable seeded once from the env, with a fallback to
the in-tree `vitriol-stream/target/release/libvitriol_stream.a` symlink, and an
explicit warning when the configured path is missing.

Verified: `env -u VITRIOL_RUST_LIB cmake .` still links from cache; cache holds
`VITRIOL_RUST_LIB:FILEPATH=.../vitriol-stream/target/release/libvitriol_stream.a`.

## Verification

- Incremental build of `build-ku-cu12` (source dir
  `/home/randozart/Desktop/Projects/VITRIOL/llama.cpp`); `libllama.so.0.4.0`
  and `llama-server` relinked 2026-09-28 16:41.
- `nm` confirms `vitriol_probe::cancel_batch()` present and `vs_*` resolved.
- **Deep-context certification (2026-09-28 16:52).** Restarted via
  `systemctl restart vitriol-server.service`, then POSTed a 371,956-char prompt
  (132,697 tokens) with MTP + `VITRIOL_KV_SCORE=probe`. Log:
  - `VITRIOL_KV_SCORE: probe active, n_kv=132864 layers=1 decay=0.90` - the
    gate engaged at depth and the MTP draft context (1 nextn layer) was the
    scored memory. This is exactly the cross-context scenario that used to
    throw.
  - `pre_decode() failed` / `unordered_map::at`: **0 occurrences**.
  - Only log errors are the pre-existing missing `/scratch/kvtier/slot0.bin`
    boot warning.
  - Turn completed cleanly (`finish_reason=length`, 24 completion tokens,
    473 s including 132k prefill).
- `llamacpp:kv_ejected_total 0` (prompt fit under `n_kv`, no eviction needed;
  the defect was in scoring, not eviction).

## Stopgap (no rebuild)

`VITRIOL_KV_SCORE_MIN_FILL=0` makes the local gate unconditionally agree with
`active()`, so no captures can orphan. Costs the ~18% shallow-decode
optimization the gate was added for. Dropping MTP also avoids it.

## Follow-ups

- Captures are now scoped to the exact `llama_kv_cache_context*` (item 1 of the
  fix), so cross-context leakage is impossible by construction.
- `vitriol_probe::has_pending_output()` removed (was undeclared/dead).
- Optional: a unit test for the probe state machine would need a fake
  `llama_kv_cache_context`; not added (no existing harness for internal
  probe state). The deep replay above is the regression check for now.
