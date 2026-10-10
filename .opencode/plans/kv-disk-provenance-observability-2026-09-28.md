# KV Disk Tier: Provenance + Observability + Miss Fixes (implementation plan)

- **Date:** 2026-09-28 21:10 (impl 21:35)
- **Status:** IMPLEMENTED (compiled clean; not enabled on the live engine yet)
- **Component:** `tools/server/server-task.{h,cpp}`, `tools/server/server-context.{h,cpp}`,
  `tools/server/server.cpp`, `tools/server/server-common.{h,cpp}`; audit tool in
  `scripts/`
- **Motivation:** `single-buffedmod-2026-09-28.md` + the KV-staleness audit. The
  Optane/KV disk tier (`--slot-save-path`) is valuable (session resume across
  restarts) and must be kept. But it stores no model identity and logs almost
  nothing, so (a) a state from a different model/quant can be silently reused,
  and (b) a genuine cache miss is invisible ("occasionally misses an obvious
  cache").

## Current behavior (baseline)

- Name = FNV-1a over `(n_tokens, first 64, last 64 tokens)`, `server-task.cpp:1960-1983`.
  Pure token key; no model/config/quant identity.
- Body = wrapper `ggpc` v1 (`magic, version, n_main, n_drft, n_pack`) + `data_main`
  + `data_drft` + serialized tokens (`server-task.cpp:1985-2075`).
- Inner state header = constant `io_magic 0xaf143cd8` + `seq_id`; then
  `llama_memory::state_write` (`llama-context.cpp:3108`, `3123-3135`, `3361-3367`).
  No hparams/name/hash.
- Boot: `disk_load()` sorts newest-first, skips entries contained in a newer RAM
  entry, and `break`s when the RAM pool is full (`server-task.cpp:2292-2347`).
- Runtime: `disk_fetch()` only runs when the RAM scan found nothing
  (`server-task.cpp:1839`), scans all files by token LCP with `f_keep >= 0.25`,
  and logs only on success (`2204-2290`).
- Trim cap = `limit_size * 8` = 96 GiB at `cache_ram 12288` (`2143`).
- Metrics: only `n_disk_spills`/`n_disk_restored` counters exist; only spills are
  surfaced (`server-context.cpp:874`, `server-task.cpp:1602`). `disk_load` logs once.

## Design

### 1. Provenance (disk format v2)

Add a fixed identity block after `magic+version`:

```cpp
struct server_prompt_cache_identity {
    uint64_t model_key;   // hash of model path stat + arch/n_layer/n_embd/n_head_kv/n_ctx_train
    uint64_t build_key;   // hash of (version string, kv types, n_stream, n_ctx) - config identity
    uint32_t n_ctx;       // context size at write time (informational)
    uint32_t type_k;      // ggml_type of K cache
    uint32_t type_v;      // ggml_type of V cache
    uint32_t n_stream;    // unified = 1, else n_seq_max
    uint32_t reserved[4]; // future-proof; zeroed
};
```

- `model_key`: computed once in `server-context` at init from
  `params.model.path` (`stat` size+mtime) + `llama_model_desc` +
  `llama_model_n_layer/n_embd/n_head_kv/n_ctx_train/n_vocab`. Cheap, no file
  content read. Distinguishes quants (file size differs) and configs. A model
  file swapped with identical size+mtime is not caught (acceptable; document).
- `build_key`: hashes the fields that change K/V semantics or layout:
  `type_k`, `type_v`, `n_stream`, `n_ctx`, `LLAMA_STATE_SEQ_VERSION`.
- Filename gains the model key for human/prune clarity and collision avoidance:
  `cache-<model_key:016x>-<token_hash:016x>.bin`. `disk_scan`'s `cache-`/`.bin`
  filter still matches; scan never parses the key (validation is from content).

`server_prompt_cache` gains `server_prompt_cache_identity identity;` set by
`server_context` after construction (`server-context.cpp:1372`).

### 2. Validation policy

On `disk_read`/`disk_read_tokens`/`disk_load`/`disk_fetch`:

- v1 file (no identity): **accepted**, counted as `n_disk_unverified`, logged at
  `SRV_TRC`. Rejected only when `VITRIOL_KV_DISK_STRICT=1`.
- v2 file: require `type_k`, `type_v`, `n_stream` equal to current; else reject
  (counted `n_disk_rejected_config`, `SRV_WRN`).
- v2 `model_key` mismatch: reject by default (`n_disk_rejected_model`,
  `SRV_WRN`); accept with `VITRIOL_KV_DISK_ALLOW_MODEL_MISMATCH=1`.
- On rejection in `disk_load`/`disk_fetch`, optionally `remove()` the file when
  `VITRIOL_KV_DISK_PRUNE=1` (default 0 = keep; audit-first).

Env knobs read once (static) via `getenv` in `server-task.cpp`.

### 3. Observability

Counters on `server_prompt_cache` (atomic, read from HTTP thread):
`n_disk_spills` (exists), `n_disk_restored` (exists), plus
`n_disk_fetched`, `n_disk_miss`, `n_disk_skipped_contained`,
`n_disk_rejected_config`, `n_disk_rejected_model`, `n_disk_unverified`,
`n_disk_pruned`, `n_disk_damaged`.

Logging:

- `disk_fetch` miss reasons at `SRV_TRC` (per-candidate lcp/keep/sim; final
  "no candidate" reason). One `SRV_DBG` summary per miss.
- `disk_fetch` success already logs; add the chosen `f_keep/f_sim` and identity.
- `disk_load` end log extended with loaded/skipped/rejected/unverified counts.

Metrics (`server_metrics` + `render_metrics` in `server-task.cpp:1500-1612`):

- `kv_cache_disk_restored_total`, `kv_cache_disk_fetched_total`,
  `kv_cache_disk_miss_total`, `kv_cache_disk_rejected_total{reason="config|model"}`,
  `kv_cache_disk_unverified_total`, `kv_cache_disk_pruned_total`,
  `kv_cache_disk_bytes`, `kv_cache_mem_entries`.

Admin endpoint (localhost only, read-only):

- `GET /cache` -> JSON `{ "mem": [ {tokens_hash, n_tokens, model_key, bytes} ],
  "disk": [ {name, n_tokens, model_key, bytes, mtime, verdict} ] }`.
  Implemented as `server_context_impl::get_cache_info()` + a `server_routes::get_cache`
  wired in `server.cpp` next to `/slots`. Disk listing reuses a scan; token/identity
  read uses the cheap header+tokens probe. Bounded output (cap N entries + totals).
- `GET /cache/lookup?hash=<fnv16>` -> whether a file with that token hash exists and
  its identity verdict; directly tests an "obvious miss".

### 4. Miss fixes

- `disk_load`: change the RAM-full `break` (`2332-2336`) to `continue` so later
  (older, possibly smaller) files still load; keep the containment skip.
- `load()` selection: unify RAM and disk candidate selection. Refactor
  `disk_fetch` into `disk_find_best(tokens) -> {path, f_keep, f_sim}` (scan-only,
  cheap) and `disk_read_best(path) -> bool` (pull). In `load()`, compute the RAM
  best and the disk best, then pick the better by the same rule
  (`f_keep` primary, `f_sim` tie-break) instead of "disk only when RAM is empty".
  Current RAM rule `f_keep_best < f_keep_cur && f_sim_best < f_sim_cur`
  (`1831`) is stricter than the disk rule (`2258`); normalize both to the disk
  rule. Log the chosen source (ram|disk).
- Keep `f_keep >= 0.25` gate (deliberate "don't trash large prompts").

### 5. Audit tool (read-only)

`scripts/vitriol-kvdisk-audit.py` (stdlib only):

- Parse `/scratch/kvtier/cache-*.bin`: wrapper (magic/version/sizes), identity
  block when v2, inner `io_magic`+`seq_id`, trailing packed tokens
  (`server_tokens::serialize` format: `LLAMA_TOKEN_NULL`, version, `uint32`
  tokens, media keys). Decode tokens.
- Recompute FNV-1a name and verify it matches the filename (v1 naming) or the
  `-<token_hash>` part (v2). Report mismatches as damaged.
- Report per file: name, mtime, bytes, n_tokens, seq_id, model_key/verdict,
  era (from `~/.vitriol/fingerprint.log` timeline).
- Summary: counts/bytes by verdict, and which files match the current model key.
- No writes; prints a table + JSON with `--json`.

## Files to touch

- `llama.cpp/tools/server/server-task.h` - identity struct, counters, new methods.
- `llama.cpp/tools/server/server-task.cpp` - v2 format, validation, logs, counters,
  `disk_find_best`/`disk_read_best`, `disk_load` fixes, `load()` selection.
- `llama.cpp/tools/server/server-common.h` - `server_metrics` fields.
- `llama.cpp/tools/server/server-context.h/.cpp` - identity computation at init,
  `get_metrics` propagation, `get_cache_info`.
- `llama.cpp/tools/server/server.cpp` - `/cache`, `/cache/lookup` routes.
- `scripts/vitriol-kvdisk-audit.py` - audit tool.

## Migration / compatibility

- v1 files remain readable (unverified) so the existing 61 GiB is not lost.
- After the first re-spill under v2, entries are self-identifying; run the audit,
  then prune incompatible/old files (`VITRIOL_KV_DISK_PRUNE=1` or the audit's
  suggested list).
- No fingerprint keys change for the live launch; identity is internal. Disk
  filenames change format (old + new coexist; trim evicts oldest by mtime).

## Out of scope / decisions deferred

- Whether to hash GGUF attention tensors for a stricter "K/V-relevant" key
  (would keep BuffedMod<->ISTA reuse). Default: strict model key.
- Lowering the 96 GiB disk multiple.
- Auth on `/cache` beyond localhost binding.

## Implementation status (2026-09-28 21:35)

Done, compiled clean (`make -j8 llama-server` in `build-ku-cu12`; only the
pre-existing `unused parameter req` warning). Not yet active: the running
engine still has the previous `libllama-server-impl.so` mapped; changes take
effect on the next rebuild + restart.

Implemented:
- `server-task.h`: `server_prompt_cache_identity`, `server_prompt_cache_disk_info`,
  identity member, disk counters, `disk_find_best`/`disk_read_best`/`disk_list`.
- `server-task.cpp`: wrapper v2 (identity block, v1 still read), identity
  verdicts + env policy (`VITRIOL_KV_DISK_STRICT`, `VITRIOL_KV_DISK_ALLOW_MODEL_MISMATCH`,
  `VITRIOL_KV_DISK_PRUNE`), model key in filename, per-outcome counters and logs,
  `disk_load` `break`->`continue` + reject accounting, and `load()` now compares
  the best disk mirror against the best RAM entry (was: disk only when RAM empty).
- `server-common.h` + metrics render: 7 new counters/gauges.
- `server-context.cpp/.h`: identity computed at init (model file stat + hparams;
  `SRV_TRC` logs model_key/build_key), `get_metrics` propagation, `get_cache_info`.
- `server.cpp`: `GET /cache` (and `?hash=` filter), proxied in router mode.
- `scripts/vitriol-kvdisk-audit.py`: read-only auditor.

Audit result on the existing tier (2026-09-28 21:34):
`49 files, 60.1 GiB, all version=1 (v1-unverified), all filename hashes OK`.
Under the new code these stay accepted (v1 policy) until `VITRIOL_KV_DISK_STRICT=1`;
new spills are v2 and self-identifying.

## Test plan (when approved, not now)

1. Build (`build-ku-cu12`) and unit-check the format round-trip.
2. Restart engine; verify `disk_load` logs identity verdicts and v1 files load
   unverified.
3. Spill a new state; confirm v2 identity written and filename has the model key.
4. `/cache` and `/cache/lookup` return expected entries.
5. Regression: same-model session resume still hits; mismatched-model state is
   rejected with a clear log.
6. `vitriol-kvdisk-audit.py` classifies the existing 61 GiB correctly.
