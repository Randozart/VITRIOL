# Incident 2026-08-23 — hard freeze under memory pressure

**Severity:** box-wide hard lock, user hard-reset required
**Duration of exposure:** ~10 minutes from pressure onset to freeze
**Status:** hardened (gateway guardrail + head caps + swappiness), one
user action applied, monitoring live in TUI

## Timeline (reconstructed from journal + access logs)

| time | event |
|---|---|
| ~18:00 | user's REBIS session active through Mercury; Sol generating (multi-minute turns) |
| ~18:1x | second opencode session running a cargo/rustc build (8 cores at 100%) |
| 18:19 | hermes kanban self-throttles: "system memory pressure is elevated" |
| 18:28:21 | journald: "Under memory pressure, flushing caches" — **journal ends mid-sentence** |
| ~18:38 | user hard-reset (system fully unresponsive) |

No OOM-kill, no panic, no NVRM/Xid in the journal: the kernel died quietly.
That absence is the diagnosis.

## Root cause chain (three layers)

1. **Anon-memory demand exceeded physical RAM + zram.** Resident: Sol+Luna
   caches/checkpoints (~2–3 GB), hermes session workers, two opencode
   instances, a cargo build, the co-tenant's processes — on 15 GB.
2. **Swap topology amplified it.** This box swaps to 8 GiB zram (compressed
   pages that *consume real RAM*) and then an 8 GiB swapfile on the root
   filesystem. When zram filled, swapfile I/O thrashed the root disk.
3. **Swappiness 60 preferred swapping live anon pages over reclaiming our
   GBs of mmap'd model weights** (clean, reloadable page-cache). Thrash
   starved even the OOM killer's execution → hard freeze with zero journal
   tail. A normal OOM-kill would have been survivable; the thrash freeze
   was not.

## Hardening applied (all committed)

| fix | commit | effect |
|---|---|---|
| Gateway memory guardrail | 090aa60 | MemAvailable < 1200 MiB ⇒ clean 503 + Retry-After: 60 — one refused turn instead of a frozen box |
| Head caps lowered | 090aa60 | cache-ram 1024/512 MiB, ctx-checkpoints 8 @ 16384 spacing (was 2048/1024, 12 @ 8192) |
| `vm.swappiness=10` | user-applied, persisted in `/etc/sysctl.d/99-rebis.conf` | kernel reclaims mmap'd model pages (reloadable) before swapping live memory |
| TUI RAM gauge | 090aa60 | REBIS tab: HOST RAM available — ok / low <1500 / FREEZE RISK <800 MiB |
| Supervisor redesign | 6b02346 | (earlier) no duplicate spawn churn truncating logs and loading models every 15 s |

## Why the freeze looked like "Rebis doing a lot but showing nothing"

The freeze window coincided with heavy gateway traffic (access lines show
multi-minute turns). The gateway was working; the box beneath it was dying.
The REBIS tab's new HOST RAM gauge exists precisely so this state is visible
*before* the freeze — FREEZE RISK appears at 800 MiB available.

## Residual risks & monitoring

- **Co-tenant killalls** still fail in-flight turns (supervisors restore in
  ~15 s + ~40 s load). Unfixable without endpoint ownership coordination.
- **Builds during serving** remain the top RAM spike source. The guardrail
  now converts that spike into refused turns rather than a freeze — but the
  build itself may still suffer. Prefer sequencing.
- **Watch the gauge**: HOST RAM `low` = finish what you're doing;
  `FREEZE RISK` = close workloads before the kernel makes that choice for
  you (it may not get the chance — that is this incident's lesson).
- If a freeze recurs despite swappiness 10 + guardrail: check `zram` usage
  (`zramctl`) — compressed-RAM swap consuming real RAM is the next
  candidate to cap.

## User actions completed

- `vm.swappiness=10` applied at runtime and persisted via
  `/etc/sysctl.d/99-rebis.conf` (verified: runtime reads 10).
- Nothing further required for persistence; the setting survives reboots.
