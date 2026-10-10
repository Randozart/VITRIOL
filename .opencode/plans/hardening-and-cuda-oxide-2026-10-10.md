# Hardening batch + cuda-oxide spike (no restart)

- **Date:** 2026-10-10 13:13
- **Status:** approved by owner 2026-10-10; execution started 13:13
- **Predecessor:** `kv-cache-cleanup-full-refill-2026-10-10.md` (stage A shipped
  inner `8dd8e3276`).
- **Constraints:** NO engine restart in this batch. Commit everything that works
  and is found functional. Repo commit style + `Assisted-by: opencode`.

## Research findings driving the plan (2026-10-10)

1. **Untracked-but-critical files** (build from working tree, lost on clone):
   - `src/models/qwen35-mtp.cpp`, `qwen35moe-mtp.cpp`, `gemma4_mtp.cpp`
     (MTP arch impl - live engine runs `--spec-type mtp`)
   - `ggml/src/ggml-cuda/vitriol_ce_d2b.cpp` (AGENTS-referenced harness)
   - `tools/mtmd/models/hunyuanocr.cpp`, `models/templates/stepfun-ai-Step-3.5-Flash.jinja`
   - `.github/**` (FUNDING, workflows, vulkan action), `.gitattributes`
   - outer: `profiles/qwen38-distributed-262k/`
   - `vitriol-stream/` crate itself IS tracked (10 files); only `target/` untracked.
2. **VITRIOL has zero custom GPU kernels** (`__global__` count = 0 in all
   `ggml/src/ggml-cuda/vitriol*.cpp`) -> cuda-oxide is greenfield capability,
   not a port. Best first kernel: **KV probe scoring GPU-ization**
   (`src/vitriol-kv-probe.cpp`, CPU today; scoring gates = ~18% shallow-decode
   penalty per `kv.score_min_fill` docs).
3. FNV-1a basis fix in `vitriol-cuda-integration.cpp:347` already committed.
   Server identity hashes (`server-task.cpp:2043`, `server-context.cpp:1447/1474`)
   keep the truncated basis **on purpose** - 32 live mirror files carry those
   keys; changing the basis orphans them all.
4. `--no-cache-idle-slots` exists (`common/arg.cpp:1737`) but `scripts/vitriol`
   never emits it -> `cis=0` fingerprint is a lie while engine runs flag ON.
5. Sidecar unit `/etc/systemd/system/vitriol-autosave.service` dead since
   Oct 3, no `VITRIOL_SLOT_SAVE_DIR` env; engine config `slot_save_path =
   /scratch/kvtier` vs sidecar default `~/.vitriol/checkpoints` = never worked.
6. No tiny gguf on host (smallest 5.9 GB) -> harness downloads Qwen2.5-0.5B
   (~350 MB) via ranged HF downloader (HF-CDN-stall doctrine, AGENTS rule 6).
7. sudo needs password -> /etc/systemd/system drop-ins staged in-repo +
   installer script, owner runs with sudo when ready.

## Stage 1 - Commit round

Inner `llama.cpp` (explicit paths, NEVER `git add -A` - in-source CMake junk):
add the 6 file groups from finding 1. Delete `tools/server/*.rebis-bak`.
Verify: incremental build no-op, `bash -n scripts/vitriol`, `serve --dry-run`.
NOT committed: in-source CMake artifacts (modified root `Makefile`,
`license.cpp`, `llama.pc`, `*-version.h`, `ggml-config*.cmake`,
`DartConfiguration.tcl`, `tools/server/webui/` = downloaded UI).

Outer: add `profiles/qwen38-distributed-262k/`, `.gitignore` +=
`vitriol-stream/target`, bump submodule pointer. Delete orphan
`/scratch/kvtier/slot0.tmp.bin` (1200 B, dead sidecar artifact).

## Stage 2 - Checkpoint rehydrate after restore (hardening B)

Gap: `common_prompt_checkpoint` ring lives in `slot.prompt.checkpoints`;
RAM-tier states carry it (`alloc` line 1826) but `disk_write` persists only
tokens+KV (`server-task.cpp` disk_write signature) -> after any disk restore
or fresh slot the ring is empty -> `do_reset` (`server-context.cpp:3498`)
guarantees full re-prefill when `pos_min >= pos_min_thold`.

Fix (no disk-format change): after a successful `prompt_load` restore,
synthesize a checkpoint covering the restored KV range using the existing
checkpoint-creation path (`update_tgt` + `update_pos`, see the
LCP-boundary checkpoint creation near `server-context.cpp:2400-2471`).
Ring depth (`--ctx-checkpoints 4`) unchanged. Expected effect: the new
`forcing full prompt re-processing` WRN goes quiet after restores.

Verification: build clean (no new warnings), review that created checkpoint
respects `PARTIAL_ONLY` flags like existing creators.

## Stage 3 - Restart-independent ops fixes

1. `scripts/vitriol`: `cache_idle_slots = off` emits `--no-cache-idle-slots`
   (currently only omits `--cache-idle-slots`, upstream default is ON);
   `cis=` fingerprint stays but now tells the truth.
2. Stage drop-ins in `scripts/systemd/`:
   - `vitriol-autosave.service.d/dir.conf`: `VITRIOL_SLOT_SAVE_DIR=/scratch/kvtier`
   - `vitriol-server.service.d/prune.conf`: `VITRIOL_KV_DISK_PRUNE=0`
     (explicit OFF: PRUNE=1 deletes all mirrors at boot on model-file
     mtime/size touch - manual audit preferred; owner concurred)
   - `scripts/install-vitriol-dropins.sh` (run with sudo later)
3. PARKED (restart day): systemd return + `OOMScoreAdjust=-500`, `/metrics`
   re-verify with new binary, first-spill trim WRN watch (cap now 144 GiB),
   sidecar unit enable/start.

## Stage 4 - kvtier smoke harness

`scripts/test-kvtier-smoke.sh`:
- fetch Qwen2.5-0.5B-Instruct Q4_K_M (~350 MB) with ranged downloader
  (reuse `/tmp/mellum2_fetch.py` pattern; verify `lfs.oid` sha256 - etag is
  xetHash, NOT sha256)
- spare port (18279), `-ngl 0` CPU (zero contention with live engine),
  temp `--slot-save-path` + `--cache-ram 512`
- tests:
  T1 warm resume: prefill -> response -> SIGKILL -> relaunch -> assert
     second request `cache_n > 0`
  T2 trim loud: `VITRIOL_KV_DISK_TRIM_X=1`, force 2 spills -> assert
     `disk trim: removed` WRN appears, files actually deleted
  T3 identity mismatch: rewrite one mirror with foreign model_key ->
     assert boot log `identity mismatch` + file retained (PRUNE off)
  T4 near-miss log: craft diverged prompt -> assert f_keep WRN or miss TRC
- exit non-zero on any failure; fully self-cleaning (tmp dirs, killed procs).

## Stage 5 - cuda-oxide spike (isolated)

1. Toolchain: `rustup toolchain install nightly-2026-08-28` + components,
   `cargo +nightly-2026-08-28 install --git https://github.com/NVlabs/cuda-oxide.git cargo-oxide`.
   Check whether CUDA 13.x is actually required (cuda-oxide emits PTX via
   LLVM - likely fine on host cuda-12.9; cutile-rs deferred, needs 13.3).
   NOTHING touches the ggml build (stays nvcc 12.9, arch 61;86).
2. New crate `vitriol-gpu-rs/` (Apache-2.0 OR MIT, PROVENANCE header,
   docs/provenance entry): hello-world -> compute-sanitizer clean.
3. Candidate kernel: **KV probe scoring on GPU** (parity vs
   `src/vitriol-kv-probe.cpp` output on recorded probe inputs).
4. Bench + adopt/reject decision doc (kernels stay out of the engine until
   adopted; engine link path unchanged).

## Answers to open questions (owner, 2026-10-10)

- garbage deletion: YES (rebis-bak + slot0.tmp.bin)
- `VITRIOL_KV_DISK_PRUNE`: explicit 0
- cuda-oxide first kernel: probe scoring
- commit style: repo style + `Assisted-by: opencode`
