# BUGS.md

Bugs and root causes are logged here per `AGENTS.md` §4.6 / §9.6. Timestamped records
are historical — reference them, never edit retroactively. Companion to
`docs/CHANGELOG_2026-08-06.md` and `docs/CURRENT_ARCHITECTURE.md`.

## 2026-08-06 — Copula / runtime bugs found and fixed

### 1. Uncommitted edge write stalled SQLite writes ~5 s (Hermetis store)
- **Where**: `libvitriol/hermetis/db.py` — `store_episode` called `_ensure_edge`
  (edge INSERT) without a commit, leaving an open write transaction that held the
  SQLite write lock.
- **Symptom**: the 3rd sequential `/hermetis/store` request stalled ~5 s
  (`busy_timeout`) under threaded Flask; direct single-thread calls masked it.
- **Fix**: commit after the edge link; `get_or_create_edge` now commits under the write
  lock. Also held `_write_lock` across the entire write (was only around the connection
  fetch).

### 2. `vitriol setup` cleared the capability it just set
- **Where**: `scripts/vitriol` — `setup_caps` (setcap) ran BEFORE `fix_rpath`
  (patchelf), which rewrites the ELF and clears file capabilities.
- **Symptom**: after `sudo vitriol setup`, `getcap` on llama-server was empty.
- **Fix**: reordered — `fix_rpath` first, `setcap` last (`a583047`).

### 3. `VITRIOL_KV_MODE=offload` aborted on CPU-placed layers
- **Where**: `llama.cpp/src/llama-kv-cache.cpp` — the offload branch called
  `ggml_backend_dev_host_buffer_type(dev)` for every layer; CPU-placed layers
  (il >= ngl) have a device whose `get_host_buffer_type` is NULL (ggml-cpu.cpp:489) →
  NULL → `ggml_backend_buft_is_host(NULL)` assert during KV cache init.
- **Fix**: fall back to the normal device buffer type when the host buffer type is NULL
  (submodule `85d01eda8`).

### 4. `get_edge_targets` UNION column mismatch after node-versioning migration
- **Where**: `libvitriol/hermetis/db.py` — `SELECT e.*, ... UNION SELECT n.*, ...`.
- **Symptom**: `sqlite3.OperationalError: SELECTs to the left and right of UNION do not
  have the same number of result columns` — exposed when P3.1 added
  `git_rev`/`superseded`/`superseded_by` to `knowledge_nodes` (column counts diverged).
- **Fix**: explicit matching columns (`_type, id, created_at, content, strength`).

### 5. Clean build fails: cpp-httplib linked without -fPIC
- **Where**: `scripts/build-llama-server.sh` — vendored `cpp-httplib` static lib is
  linked into `libllama-common.so`; a clean configure failed with
  "relocation R_X86_64_TPOFF32 ... recompile with -fPIC".
- **Fix**: `-DCMAKE_POSITION_INDEPENDENT_CODE=ON` added (`cfa0ef9`).

### 6. Stale-build artifacts (two instances)
- **BERT zero embeddings** (P2-era binary): "fast"→norm 0.0, "hello world"→1.0,
  backend- and pooling-independent. **NOT a current-source bug**: a clean rebuild
  produces correct embeddings (norm 1.0, all inputs). Root cause: stale P2-era server
  processes (killed). GGUF-GPU embeddings verified working; backlog closed.
- **Decode regression** (full-launch test): 8–15 t/s vs ~30 baseline. Root cause:
  stale incremental build dir. Clean rebuild yields **37–38 t/s** (better than baseline).
- **Lesson**: after heavy testing/edits, do a clean rebuild (`rm -rf build` + the PIC
  flag) before trusting any measurement.

### 7. Bad model file (not a fork bug)
- `qwen3.6-35b-a3b-instruct-TQ1_0.gguf` produces garbage in every mode (stream + CPU);
  the dense BitNet TQ1_0 substrate is fine in the fork. The file is suspect (bad
  conversion / vision-model tokenizer). Stream/VITRIOL-knob sweep deferred behind a
  known-good stream-requiring model.

## 2026-08-06 — tooling issues (not runtime bugs)

- **Praetor intent rule** rejects valid Python `#` comments (verified with a minimal
  file) — disabled via `.praetor.toml` `[intent] enabled=false` (maintainer decision).
- **Praetor pre-commit hook** flagged the whole repo baseline (38k+ diagnostics) —
  scoped to changed files, then made **diff-aware** (`scripts/praetor_diff.py`): only
  NEW diagnostics relative to the staged version gate commits.
- **fish shell** (`sudo prlimit --pid $$ ...`) — `$$` invalid in fish; use
  `$fish_pid` and `; and`.

### 8. `vitriol stop` failed + launches appeared broken (port phantom + set -e aborts)
- **Where**: `scripts/launch_vitriol_full.sh` / `launch_copula.sh`.
- **Symptom**: `vitriol stop` exited 1 without killing the gen server; launches reported
  "exited immediately" while the model was actually still loading; a stale gen server
  held :8279 invisibly (ss -p/lsof/fuser showed owner "-", inode unclaimed in /proc).
- **Root causes**:
  1. `port_pid` relied on `ss -p`, which cannot attribute pids for our own `setsid`
     servers; the pipeline returning non-zero on a miss **aborted the script under
     `set -euo pipefail`**.
  2. Launch hardening polled for the **port binding**, but the port binds only after the
     ~50 s model load → a healthy loading server was misread as dead.
  3. `log_err` matched the benign "failed to fit params" warning as a fatal error.
- **Fixes**: `port_pid` now falls back ss -> lsof -> **pgrep (cmdline `--port`)** -> fuser
  and always returns 0; hardening polls **process liveness** (`kill -0`), not binding;
  `log_err` uses fatal-marker patterns only; launch treats a healthy port as
  already-running; `stop` kills by pid (or socket as last resort).
