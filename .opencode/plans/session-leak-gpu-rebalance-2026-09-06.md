# /new Session Leakage Fix + GPU Layer Rebalance

**Date**: 2026-09-06
**Status**: IMPLEMENTED
**Owner**: randozart
**Trigger**: owner reports (1) cross-session task/scratchpad leakage only when using
`/new` in Ontic (2) 1070 Ti pressure greater than 3060.

---

## Problem 1 — `/new` session leakage

### Symptom

Typing `/new` in Ontic starts a new session, but the sidebar (and context injection)
still show the PREVIOUS session's tasks and scratchpad notes.

### Root cause (verified against pi-coding-agent dist)

`/new` → `RpcCommand::NewSession` → pi `runtimeHost.newSession()` **in the same
process** (no respawn). Sequence:

1. `teardownCurrent()` emits `session_shutdown` — **neither task-state nor scratchpad
   had a handler**, so module-level `currentTaskFile` / `currentFile` were never reset.
2. `finishSessionReplacement()` → `rebindSession()` → `bindExtensions()` fires
   `session_start` on all extensions **in filesystem load order**:
   `scratchpad` → `session-panel` → `task-state`.
3. `session-panel`'s `session_start` handler calls `renderSidebar()`, which reads
   `currentTaskFile` **before** `task-state`'s handler has updated it → one full
   render cycle with the previous session's file.
4. rpc-mode calls `rebindSession()` AGAIN (double `session_start`) — race repeats.

Note: fresh TUI launches are NOT affected — extensions load fresh, module state
initializes before any session_start.

### Fix

- `session_shutdown` handlers in **both** `task-state/index.ts` and
  `scratchpad/index.ts` that reset module-level file paths to defaults. Clean state
  before the next session's events; the read-before-write race window then yields
  empty (not stale) data.
- `requestSidebarUpdate()` at the END of task-state's (and scratchpad's)
  `session_start` handler — re-render AFTER the module vars are correct, healing the
  session-panel-reads-early race without architectural change to the event bus.

---

## Problem 2 — 1070 Ti VRAM pressure

### Measured (2026-09-06 13:12, idle engine, ts 22,14)

| | GPU 0 — RTX 3060 (12 GB) | GPU 1 — GTX 1070 Ti (8 GB) |
|---|---|---|
| Used | 9,207 MiB (74.9%) | 8,050 MiB (**98.3%**) |
| Free | 2,687 MiB | **58 MiB** |

`ts 22,14` proportional split over 65 layers → 40 layers GPU 0 / **25 layers GPU 1**.
Fixed per-GPU overhead (CUDA ctx ~200 MiB + compute buffers ~500-800 MiB + KV 6 attn
layers ~480-800 MiB) eats ~1.6 GB of GPU 1's 8 GB → knife edge.

### Fix — `ts 22,14` → `ts 26,10`

| | Before | After |
|---|---|---|
| GPU 1 layers | 25 | **18** |
| GPU 1 VRAM | 98.3% / 58 MiB free | **~81% / ~1,540 MiB free** |
| GPU 0 layers | 40 | 47 |
| GPU 0 VRAM | 74.9% | ~78% (~2,100 MiB free) |
| Decode | ~14-16 t/s | **~15-17 t/s (+5-10%; faster card gets more layers)** |

ts 26,10 chosen over 27,9: 27,9 puts GPU 0 at ~82% with only ~1,700 MiB headroom —
uncomfortably close to the depth wall (VRAM creep ~23 KiB/token on dev0 during long
prefills, per AGENTS.md). ts 26,10 is battle-tested (`qwen38-mtp-131k` profile,
12.89 t/s shallow-bench).

MTP stays ON (n=1): +40% decode shallow / +31% at depth; its ~160 MiB draft head
lives on GPU 0 which has headroom. The problem was layer distribution, not MTP.

### Steps

1. `~/.vitriol/config`: `tensor_split = 22,14` → `26,10`
2. `vitriol config bless` (fingerprint diff against previous — flag-provenance rule)
3. `systemctl restart vitriol-server.service`
4. Verify `nvidia-smi`: GPU 1 ~6.5-7.0 GiB (~81%)
5. Shallow bench 3×64 to confirm t/s
6. Record in EXPERIMENT_LOG.md

---

## Verification

- `npx vitest --run officina/.pi/extensions/` — 47 files / 535 tests green
- `/new` in Ontic: sidebar tasks + scratchpad rows EMPTY after switch; importable
  via `/import` (2026-09-06, be2a641)
- Engine restart via unit; fingerprint carries `ts=26,10`; `ckpts=4` `cache_ram=256`
  etc. unchanged
