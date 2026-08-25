# Watchdog & Context-Persistence Plan — Surviving OOM on a 16 GiB Host

> **Date:** 2026-08-25
> **Trigger:** Second OOM kill in two hours (20:18:12, anon-rss 4.9 GiB, dual-slot
> c=98304). Root cause is host-level: opencode footprint (~5.2 GiB across 6
> processes) + DDR3 ceiling; the server is merely the kernel's preferred victim
> (`oom_score_adj` 200). Investigation evidence: `dual-slot-report-2026-08-25.md`
> follow-up session log; VRAM/persistence audit (this session).

## 1. Facts established

| fact | source |
|---|---|
| `CUDA_CHECK` is `[[noreturn]]` → any VRAM alloc failure aborts instantly | `ggml/src/ggml-cuda/common.cuh:155`, `ggml-cuda.cu:97` |
| No NVML / free-VRAM monitoring anywhere | grep: zero hits |
| VITRIOL pools (output cache, pin, LRU) degrade gracefully; core paths do not | `vitriol-cuda-integration.cpp:130/718/835` |
| Only restart-surviving KV persistence = `--slot-save-path` + `POST /slots/{id}?action=save/restore` (`llama_state_seq_save_file`) | `server-context.cpp:2111-2197`, `llama-context.cpp:3163` |
| `--cache-idle-slots`, `--cache-ram`, `--ctx-checkpoints`: RAM-only, die with process | audit table |
| SIGTERM handler does NOT save state — just unblocks loop | `server.cpp:322`, `server-context.cpp:3478` |
| llama-server runs via `nohup` — nothing restarts it after OOM | `scripts/vitriol:2143/2200` |

**Answer to "shouldn't it just go OOM if another process spikes VRAM?"** — it's
worse: a VRAM steal doesn't OOM politely, it hits a `[[noreturn]]` CUDA_CHECK
abort mid-graph-build. There is no graceful path and no detection. Mitigation
is therefore preventive (OOM score) + reactive (restart + restore), not graceful.

## 2. Design

```
┌─ systemd --user ────────────────────────────────────────────────┐
│ vitriol-server.service                                          │
│   ExecStartPre = vitriol stop          (kill stale, AGENTS §3)  │
│   ExecStart    = vitriol serve --detach (--slot-save-path set)  │
│   Restart=always  RestartSec=5                                  │
│   OOMScoreAdjust=-500  ← kernel now prefers opencode/firefox    │
│                                                                 │
│ vitriol-autosave.service                                        │
│   lull_slot_persist.py:                                         │
│     wait health → POST /slots/N?action=restore slotN.bin        │
│     loop: every VITRIOL_AUTOSAVE_SECS (300)                     │
│       for idle slots with context: POST action=save             │
└─────────────────────────────────────────────────────────────────┘
fallback (non-systemd hosts): scripts/vitriol-watchdog.sh (health poll)
```

Decisions:
- **No MemoryMax/MemoryHigh on the server unit.** The server is not the hog;
  a cgroup cap would manufacture a new kill vector during legit prefill spikes.
  Protection comes from OOMScoreAdjust alone.
- **Fixed filenames** `slot{ID}.bin` (ring of one per slot): stale-file rot is
  impossible; each save overwrites.
- **Restore-on-start lives in the autosave service**, keeping the server unit a
  pure wrapper around the existing tested launcher path.
- **Context budget cut to c=81920** (dual split `0=73728,1=8192`): frees ~0.4 GiB
  VRAM+host pressure; hermes provider `context_length` follows to 81920 so it is
  correct under BOTH profiles (master single-slot = full 81920).
- **Data-loss bound** = autosave interval (≤5 min of conversation tail). OOM has
  no graceful window; periodic save is the only honest contract.

## 3. Steps

1. Launcher: `[server] slot_save_path` key → `--slot-save-path` arg + fingerprint `ssp=1`.
2. Profiles (repo + installed sync):
   - master: context 98304→**81920**
   - ontic: context **81920**, slot_context `0=73728,1=8192`, slot_save_path
   - metas updated; hermes yaml VITRIOL context_length → 81920.
3. `scripts/lull_slot_persist.py` — urllib-only; waits `/health`; restores
   `slot{N}.bin` into slot N (errors tolerated: model-mismatch, empty); then
   periodic save of idle non-empty slots. Env: PORT, INTERVAL, DIR.
4. `scripts/vitriol-watchdog.sh` — fallback health-loop restarter.
5. Canonical units in repo `systemd/user/`, installed to `~/.config/systemd/user/`,
   `daemon-reload`, enable+start both.
6. Cutover: stop nohup instance → start services → verify: unit active,
   gen-log fingerprint fresh, restore attempt logged, forced save produces
   slot0.bin, kill -9 server → auto-restart ≤10 s → restore replays.
7. Commits + report addendum (crash postmortem #2, watchdog architecture).

## 4. Verification matrix

| test | expected |
|---|---|
| `systemctl --user status vitriol-server` | active, main PID = launcher child |
| `kill -9 $(pgrep llama-server)` | service restarts server ≤ ~35 s (model load) |
| autosave log after first tick | `saved slot0.bin (n tokens)` |
| restart with warm bin present | `restored slot0.bin` before first request |
| hermes turn after restart | prefix cache hit / short prefill, no tiny-ctx complaint |
| oom_score_adj of server | `-500` |

## 5. Rollback

`systemctl --user disable --now vitriol-{server,autosave}` then relaunch via
`vitriol serve --detach`. Profiles revert by restoring context lines. Units are
additive; no upstream code changes required for this plan.
