# Monorepo engine import + server rename (2026-10-23)

Branch `engine-monorepo` (off `stack-pull`). Plan approved by owner:
absorb the fork in-tree, rename the server, freeze upstream sync.
GPUs busy - **no engine runs, no cutover**; build+CPU checks only.

## Decisions (owner, 2026-10-23)

- History: **squash snapshot** (easiest+cleanest given frozen upstream;
  fork repo + bundle remain the archaeology source).
- Directory: **`engine/`** (renamed from `llama.cpp/`).
- Binary: **`vitriol-server`** (CMake target stays `llama-server`,
  compat symlink in `engine/build/bin/`).
- Upstream: **FROZEN** (manual ports only).

## S0 - prep DONE
- Branch `engine-monorepo` off `stack-pull`.
- Fork tagged `engine-import-2026-10-23` at `30b128504`.
- Bundle `~/Backups/vitriol-engine-2026-10-23.bundle` (405M, all refs).
- Gitdir pointer backed up (`~/Backups/engine-gitfile-2026-10-23.txt`).
- Source tree hash recorded: `058c67dc9245a540d91981d7db7fdd2d620672f4`.

## S1 - absorb DONE (commit `0e32743`)
- `git rm --cached llama.cpp`; `.gitmodules` removed; local submodule
  config section removed; `mv llama.cpp engine`; nested `.git` pointer
  removed.
- Staged via **exact ls-tree file list** (first `git add engine` attempt
  swept 773 untracked junk files - caught by the 3600-count gate, reset,
  redone).
- Verification BEFORE commit: staged count == 3600; `git write-tree` ->
  `<tree>:engine` == source tree hash (exact match); no gitlink (0 x
  mode 160000) after fixing a gitlink resurrection caused by
  `git reset` (HEAD still carried it) - `git rm --cached` + amend.
- Working tree: 14 GB of ignored `build-*` dirs moved with the rename
  (still ignored by the nested `.gitignore`).

## S2 - rename DONE
- `engine/tools/server/CMakeLists.txt`: `OUTPUT_NAME vitriol-server`
  (target stays `llama-server`).
- `scripts/build-llama-server.sh`: engine path (`VITRIOL_ENGINE_DIR`,
  legacy `VITRIOL_LLAMA_DIR`), post-build compat symlink
  `llama-server -> vitriol-server`.
- `scripts/vitriol`: `ENGINE_DIR` (+ alias), SERVER_BINARY prefers
  `vitriol-server` w/ fallback; pidfile now `vitriol-server.pid`, stop
  falls back to legacy `llama-server.pid`; display strings.
- Process-name detectors accept BOTH names (transition window):
  governor `engine.rs` (`pids_by_comm` x2), sidecar
  `lull_slot_persist.py` (`server_pid` set + `PROTECTED`), watchdog
  (killall x2), oom-hardening (ERE pgrep + unit Description), repo user
  unit Description.
- `scripts/apply-llama-patches.sh`: DEPRECATED header.
- Root symlink `llama.cpp -> engine` (gitignored) keeps legacy absolute
  paths + path-based legacy scripts alive.

## S3 - docs DONE
- `docs/ENGINE_PROVENANCE.md` (import record + frozen policy).
- `docs/ARCHITECTURE.md` (layers, ownership rules, process-name
  contract, changelog).
- `NOTICE` (upstream MIT attribution).
- `AGENTS.md` (monorepo conventions, killall name, licensing wording).
- `AGENT_BRIEF.md` gotcha #1 rewritten.

## S4 - CPU verification
- `scripts/vitriol config show`: exit 0 (config/fingerprint parsing OK).
- `cargo test` vitriol-governor: 7/7 (dual-comm change).
- `cargo build --release` vitriol-governor: OK (cutover installs it).
- Fresh CUDA configure+build (target `llama-server`) -> **BUILT, 0
  errors**; `engine/build/bin/vitriol-server` exists and `--version`
  runs (v0.4.0-dev, build 739); compat symlink
  `engine/build/bin/llama-server -> vitriol-server` in place.
  NOTE: `engine/build` has been a SYMLINK to `build-ku-cu12` since
  Aug-31; the configure reused that cache and the `llama.cpp -> engine`
  source-path change forced a full recompile inside it. This is the same
  tree the launcher has always used, so the verification is
  representative - but cutover should explicitly confirm whether to
  keep building through the symlink or make `engine/build` a real dir.
- Reference audit: remaining `llama-server` mentions in `scripts/vitriol`
  are comments/labels + the intentional fallback; path-based legacy
  scripts covered by the root symlink.

## S5 - GPU-window cutover (NOT DONE - owner runs it)
1. `sudo systemctl stop vitriol-server.service`
2. `scripts/build-llama-server.sh --backend cuda` (fresh `engine/build`,
   archs 61;86, compat symlink + caps reapplied)
3. `sudo install -m755 vitriol-governor/target/release/vitriol-governor
   /usr/local/bin/vitriol-governor && sudo systemctl restart
   vitriol-governor.service` (running binary still scans for
   `llama-server` only until this step)
4. `sudo systemctl start vitriol-server.service`
5. Gates: /health, /props, one completion, /metrics new series
   (ttft/itl/queue/lcp/graph), warm slot restore, governor sees engine
   PID, sidecar sees engine, TUI online.
6. Update `/etc/systemd/system/vitriol-server.service` Description
   (cosmetic; unit invokes the launcher).
7. After a validation window: close transition (drop fallbacks +
   `llama-server` symlink + root `llama.cpp` symlink), prune
   `.git/modules/llama.cpp` (reclaims ~474 MB), `git gc`.

## Rollback
- Tag `pre-engine-import-2026-10-23` to be set on `stack-pull` before
  cutover (SET - see below).
- Bundle + `.git/modules/llama.cpp` retained until post-cutover
  verification.
- `git reset --hard pre-engine-import-2026-10-23` +
  `git submodule update --init` restores the submodule era.
