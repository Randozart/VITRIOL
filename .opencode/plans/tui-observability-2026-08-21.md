# TUI Observability: serve sessions, dual GPU, decode progress

**Date:** 2026-08-21 16:00 → verified 2026-08-21 17:10
**Status:** DONE — all items implemented, 151 tests pass, live-verified against `vitriol serve --detach -port 8281`
**Scope:** `scripts/vitriol`, `scripts/launch_vitriol_full.sh`, `vitriol-tui/src/{nvidia,model,poller,ui}.rs`, `vitriol-tui/src/officina/mod.rs`

## Problem

1. TUI observes only TUI-launched stacks. `vitriol serve --detach` logs to
   `~/.vitriol/server.log` / `llama-server.log` (scripts/vitriol:2116,2170) while
   the TUI tails `${COPULA_LOG_DIR:-/tmp/opencode}/vitriol_gen.log` — so log
   tail, decode-heartbeat t/s (server-context.cpp:3234) and `[PERF]` breakdown
   never light up for serve sessions. Port also assumed via `VITRIOL_GEN_PORT`
   (default 8279); serve honours `server.port` from `~/.vitriol/config`.
2. Single GPU shown: `nvidia.rs:30-33` takes `.lines().next()` of nvidia-smi —
   RTX 3060 + GTX 1070 Ti shows only GPU 0.
3. No decode progress: server exposes `/slots` (`--slots`: per-slot
   `is_processing`, `next_token.n_decoded/n_remain`, `id_task`),
   `/metrics` (`--metrics`: totals, avg t/s gauges), `/props` draft/MTP
   acceptance (`draft.acceptance_rate`) — none launched with these flags,
   none polled.

## Decisions

- serve logs → `$COPULA_LOG_DIR/vitriol_gen.log` (single canonical telemetry path; pid files stay in `~/.vitriol`). User-selected.
- Port auto-discovery via `/proc/*/cmdline` scan when configured port fails. User-selected.
- Per-slot progress bars in DECODE card, idle slots hidden. User-selected.
- No C++ changes.

## Work items

### A. scripts/vitriol (serve)
- Both detach branches redirect llama-server output to
  `${COPULA_LOG_DIR:-/tmp/opencode}/vitriol_gen.log`; mkdir -p.
- Add `--slots --metrics` to llama-server args (both memory-mode internal and direct).

### B. launch_vitriol_full.sh
- Add `--slots --metrics` to its llama-server invocation.

### C. vitriol-tui port discovery (poller.rs)
- On `/health` failure at configured gen port: scan `/proc/[0-9]*/cmdline`
  for processes whose argv contains `llama-server` and a `--port N` arg;
  adopt first hit as gen port for subsequent polls; surface discovered port.
  Also finds memory-mode internal server on PORT−1.

### D. Multi-GPU (nvidia.rs, model.rs, ui.rs)
- Summary query adds `index,uuid`; parse ALL lines → `Vec<GpuSnapshot>`
  (new `index` field).
- Compute-apps query adds `gpu_uuid`; attribute per-GPU by uuid join;
  unmatched rows go to GPU index 0 fallback... no — keep a global list too:
  process table shows all rows with a GPU column.
- `Snapshot.gpu: Option<GpuSnapshot>` → `gpus: Vec<GpuSnapshot>`.
- GPU tab: stacked per-GPU gauge sections + merged process table w/ GPU col.
- Dashboard GPU card: one compact row pair (VRAM+UTIL) per GPU.
- Officina journal + subsystems: aggregate VRAM across GPUs.

### E. Progress + metrics (poller.rs, model.rs, ui.rs)
- Poll `GET /slots` → `SlotsSnapshot { id, id_task, is_processing, n_decoded,
  n_remain, n_ctx }[]`.
- Poll `/props` → `draft.n_accepted/n_total/acceptance_rate`.
- Poll `/metrics` (prometheus text) fallback totals: tokens_predicted_total,
  prompt_tokens_total, predicted_tokens_seconds, requests_processing.
- GEN card: `mtp acc` line when draft data present.
- DECODE card: per-active-slot braille progress bar
  `task 42 ▮▮▮▯▯ 123/512 tok` above existing velocity bar; footer slot counts
  (processing/idle) once slots known.

### F. Tests
- Parser unit tests: multi-line nvidia-smi CSV, compute-apps uuid join,
  slots JSON, props draft JSON, prometheus text, cmdline scan (fixture dir).
- `cargo test` in vitriol-tui; manual run vs detached serve.

## Risks

- `/slots` payload includes full params/prompt text unless server runs
  slots_debug==0 default (only_metrics=true) — confirmed default path omits
  prompt/generated text (server_slot::to_json(slots_debug == 0)).
- nvidia-smi absent/non-NVIDIA: Vec empty → cards render "unavailable" as today.

## Findings during implementation (2026-08-21)

1. **`next_token` is a one-element ARRAY in /slots** (`[{"n_decoded":..}]`),
   not an object — this fork's `slot.to_json` wraps it `{ {...} }`. Parser
   handles both shapes (poller.rs `parse_slots`).
2. **Heartbeat 1e6 t/s artifact**: sub-millisecond 1-token decodes emit
   `1000000.00 tokens per second (live)` (server-context.cpp:3236 division).
   Poller now ignores rates ≥ 1000 t/s (`DECODE_SPEED_SANITY_CAP`).
3. **Block-buffered stdout under nohup**: heartbeat lines flush in bursts, so
   offset-freshness flickers to 0 mid-decode. Poller replays the last sane
   rate while `/slots` reports any busy slot (`last_live_t_s`), zeroing only
   when all slots go idle.
4. Optional follow-up (not done): `setvbuf(stdout, nullptr, _IOLBF, 0)` in the
   server would line-buffer logs for ALL consumers (LOGS tab latency too);
   skipped to avoid a C++ rebuild cycle this session.

## Verification (live, GTX 1070 Ti + RTX 3060)

- `vitriol serve --detach -port 8281` → log at `/tmp/opencode/vitriol_gen.log`,
  `/slots` `/props` `/metrics` all answer.
- TUI run with `VITRIOL_GEN_PORT=9999` (wrong on purpose): GEN card shows
  `port :8281 (discovered)`, `mtp acc 90%`, `decode 15.3 t/s`.
- GPU card: `[0] RTX 3060` + `[1] GTX 1070 Ti`, separate VRAM/UTIL gauges,
  llama-server attributed per-GPU via uuid (8.9 + 6.9 GiB).
- DECODE card mid-generation: `task 1096 ▮▮▮ 28/300 tok` → `122/300` →
  `178/300`, velocity steady `15.3 t/s peak 15.3` across frames.
- Footer: `slots 1/1` while busy.
- `cargo test`: 151 passed; clippy clean; `bash -n` clean on both scripts.
