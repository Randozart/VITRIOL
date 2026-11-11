# Officina multi-session hardening — 2026-11-11

Owner-reported triad: **(1) sessions end up corrupted despite sufficient
context cache, (2) prefill is regularly retriggered, (3) dynamic routing
doesn't engage.** Investigation found the three symptoms share one root:
multi-session operation bolted onto single-session assumptions.

Supersedes the routing-only remainder of
`multi-session-routing-2026-11-10.md` (P1/P3 landed; this fixes why they
never fire). Complements `outstanding-work-survey-2026-11-11.md`.

## Diagnosis (evidence-verified)

### Symptom 1 — session corruption

- `officina/tui/src/session_guard.rs:47-61` `acquire_lock_path` writes the
  PID into `~/.vitriol/officina/tui.lock` and **never reads it back** — no
  liveness check, no refusal. `main.rs:113` calls it and ignores the
  result beyond `Some/None`.
- `prepare_session_args` (`session_guard.rs:441-448`) auto-resumes
  `latest_session(cwd)`. Two TUIs in one cwd resolve to the SAME `.jsonl`.
  pi appends via unlocked `appendFileSync`
  (`pi-coding-agent/dist/core/session-manager.js:813`) → interleaved
  divergent parent chains → `validate_session` fails → **quarantine
  renames a file a live pi is still writing** (`session_guard.rs:260-265`)
  → pi re-creates the original path on its next append → orphaned entries
  → fails validation again → quarantine loop. Operator sees "sessions
  keep ending up corrupted".
- Live proof at investigation time: 2 `officina` PIDs (3260876, 3260885),
  both cwd `/home/randozart/Desktop/Projects`; 4 `.corrupt-*`/`.repaired-*`
  artifacts; 17 session files touched in 3 days.
- Amplifiers:
  - `vitriol-checkpoint/client.ts:36` hardcodes **slot 0** for all
    sessions. With `parallel=1` the engine assigns slots by LCP/LRU
    (`engine/tools/server/server-context.cpp:1777-1846`), so session A's
    `turn_end` autosave can snapshot session B's KV and a crash-restore
    can load foreign KV ("responses referencing the other conversation").
  - `snapshot/snap.ts:32-40` writes git refs `refs/trismegistus/turns/<n>`
    with **per-session** turn indexes → concurrent sessions clobber each
    other's refs; `snapshot_rewind` can restore a foreign tree.

### Symptom 2 — prefill retriggered

Engine reuses KV only on **exact token prefix** continuation
(`server-context.cpp:3632`, `server-common.cpp:680`) — one changed token
truncates there. Two Officina extensions rewrite **mid-history**:

1. `session-ledger` — `ledger.ts:79-99` `upsertLedger` REPLACES its
   `lc-ledger` custom message wherever it sits; `index.ts:35-48` calls it
   on every `context` pass and the block (msgs/tokens/files) changes
   every turn. The ledger stays at its original near-top injection
   position → **full tail re-prefill every turn**.
2. `tool-result-clearer` — `index.ts:169-212` stubs tool results beyond
   `keep=12`; `_shared/cache-mode.ts:75-83` defers only for cloud
   providers → **always clears on `llamacpp`** → growing re-prefill as
   the window slides.

Engine-side amplifier: live profile `qwen38-buffedmod-262k-cache8-mtp2`
runs `cache_ram=8192` MiB while measured slot states serialize to
2.5–7 GiB (kv-unified bloat) → `prompt_save` alloc-skip
(`server-task.cpp:1929-1934`) means the state exists NOWHERE — the Optane
spill (`server-context.cpp:339`) sits inside the success path and is
never reached. Plus consume-on-load (`keep_restored=false` default,
`server-task.h:737`) erases restored entries. Decision: measure first
(`VITRIOL_CACHE_KEEP=1` + log attribution) before any `cache_ram` change.

### Symptom 3 — routing dormant

- Default model is the virtual `officina/lapis`;
  `adaptive-router/index.ts:245-247` returns early on virtual models, and
  `resilient-default` (which owns lapis) calls `decideTier` with no
  `peers`/`freeSlots` and **never touches the registry**. Every default
  session sees `peers=[]` → rank 0 → "senior — keeps the slot" → the
  shipped P1/P3 arbitration never engages in the default topology.
- `overflow.ts:36` `maxSwitches=2`, incremented in BOTH directions,
  reset only on `session_start` → sessions go permanently inert after
  two contention events (misreported as "cloud unavailable").
- `OFFICINA_SESSION_ROLE=subagent` is set in children
  (`subagent/spawn.ts:142`) but children load only
  `llama-cpp-provider` + `tool-gating` (`spawn.ts:311-314`) — the router
  never loads there → dead flag; peer-policy rule 3 unreachable.
- `_shared/engine.ts:185,247` — `/slots` is queue-backed; a TUI that has
  never seen one successful poll has `slots=[]` → `peersUsable=false`
  → peer arm withheld exactly under contention.
- `routing-registry.ts:184-194` read-modify-write whole file, no lock →
  concurrent TUIs lose each other's entries (last-writer-wins).

## Fix plan (stages, commit per stage)

### Stage 0 — land pending tree work
The survey's 3-file diff (divider + FreeInference lanes) had already
landed as `811d067`; the working tree instead held a broken half-landed
`/tree`+`/fork` (another session's). Completed (render_tree_modal,
key capture, session_tree_lines + tests, modal reset on
fork/switch_session) and committed. Gate: `cargo test` + fmt + rebuild
+ install binary.

### Stage 1 — cache-safe prompt
- `session-ledger`: replace in-place upsert with **remove-then-append at
  tail** each `context` pass → break point sits just before prompt end;
  re-prefill cost drops to the ledger block (~100 tok).
- `tool-result-clearer` + `_shared/cache-mode.ts`: on `llamacpp`, defer
  stubbing until ctx fill > 90% (mirror the cloud rule).
- Kill switches unchanged (`OFFICINA_NO_LEDGER`,
  `LITTLE_CODER_NO_CLEAR_TOOL_RESULTS`).
- New test: prefix-stability — context-event output across consecutive
  turns shares a growing token prefix.
- Experiment note (no code): 1-day `VITRIOL_CACHE_KEEP=1` + log
  attribution (`prompt_tokens_cached`, `forcing full prompt
  re-processing`) before touching `cache_ram`.

### Stage 2 — session-file guard
- `session_guard.rs`: real `flock` held for TUI lifetime + PID liveness
  check. Second TUI in same cwd: refuse auto-resume → start new session
  with a status notice. Quarantine/repair refuse on files held by a live
  writer. `LockGuard::drop` can no longer un-shield (kernel-held).
- `vitriol-checkpoint`: restore only own session-stem files; autosave
  skips when the routing registry shows >0 live peers.
- `snapshot`: refs namespaced `refs/trismegistus/turns/<stem>/<n>`.
- Tests: lock contention, quarantine-vs-live-writer, checkpoint stem
  match, snapshot namespacing. Gate: `cargo test` → rebuild + install
  binary (AGENTS.md officina-rework rule).

### Stage 3 — routing actually routes
- `resilient-default/index.ts`: gain `peers`/`freeSlots` via
  `_shared/engine.ts` + routing-registry; apply `peer-policy.ts` peer
  arm; write registry entries. `adaptive-router` unchanged for
  non-virtual models.
- `routing-registry.ts`: per-PID shard files
  `~/.vitriol/officina/routing-registry.d/<pid>.json`; readers
  glob-merge (lock-free, kills the lost-update race). Legacy
  `routing-registry.json` read-compat on migration.
- Switch cap: peer demotions exempt; cap resets on successful local
  return + on idle (`overflow.ts`, incl. lapis `LapisState`).
- `subagent/spawn.ts`: parent writes a `role:"subagent"` registry entry
  for the child at spawn, removes at exit (no router in the child).
- `_shared/engine.ts`: on `/slots` stall, fall back to last-known
  `slotCount` instead of withholding `peersUsable`.
- Tests: extend `overflow-peer.test.ts` + `peer-policy.test.ts`;
  shard-concurrency test; jiti-isolation entries for new `_shared` state.

### Stage 4 — docs + verification
`docs/officina-guide.md` "Running multiple sessions" refresh; this plan
gets a verification addendum with command output + fingerprints.

## Invariants

- **I1 (parity)**: 0 peers ⇒ routing **tier decisions** identical to
  today; pinned by the golden decision matrix in `peer-policy.test.ts` /
  `overflow-peer.test.ts`. AMENDMENT 2026-11-11 (stage 3): a
  stay-local reason string may name the actual blocking gate
  (cap/budget/no-candidate) when the TIER is unchanged — the old string
  blamed the cloud during cap exhaustion and misled diagnosis.
- **I2**: at most one live pi writer per session file, ever — pinned by
  the lock-contention test (`session_store_lock_lifecycle_and_peer_refusal`).
- **I3**: quarantine/repair never mutate a file held by a live writer —
  pinned by the same test (peer-held store ⇒ file left in place).
- **I4 (prefix stability)**: turn N+1's engine-visible prompt extends
  turn N's prefix; pinned by the ledger prefix-stability test + the
  provider-independent `shouldDeferToolClear` tests.
- **I5**: checkpoint save/restore never crosses sessions (stem check +
  peer gate) — pinned by `markerMatchesSession` + `checkpointPeerGate` tests.
- **I6**: rewind restores only own session's code + KV — pinned by the
  snapshot namespacing tests (same turn index, different stems ⇒ both
  refs survive).
- **I7**: concurrent registry writers never lose peer entries — pinned
  by the shard-mode test (`concurrent publishers never lose entries`).

## Verification (2026-11-11 21:30–21:50)

All gates green; commits in order:

| stage | commit | gate |
|---|---|---|
| 0 | `09ceddc` | /tree+/fork completion — cargo test 134 ✓ (found + fixed subtree-skip bug), fmt ✓, release binary installed `~/.local/bin/officina` 21:26 |
| 1 | `2c843f1` | npm test 787 ✓ (adds ledger prefix-stability + local defer tests), build-patch canaries exit 0 |
| 2 | `2d6d7f5` | TUI flock guard — cargo test 135 ✓ (adds lock lifecycle + peer-refusal test), fmt ✓ |
| 2 | `00d0bd2` | checkpoint stem/peer gate + snapshot namespacing — npm test 794 ✓, canaries exit 0 |
| 3 | `d4faaaa` | routing wired — npm test 800 ✓ (adds shard-mode I7 suite), canaries exit 0 |
| gate | — | binary rebuilt + renamed over `~/.local/bin/officina` 21:41 (running TUIs keep the old inode until exit; new launches get the guard) |

Findings corrected along the way:
- Stage 0's original content (divider relocation + FreeInference lanes)
  had already landed as `811d067` by a concurrent session; the tree
  instead held a BROKEN half-landed `/tree`+`/fork` (missing
  render_tree_modal) — completed + committed as `09ceddc` with owner
  approval.
- The `/tree` flatten skipped entire subtrees under entry-less wrapper
  nodes (found by the new unit test before it shipped).

Follow-ups (not in scope here):
- Engine-side cache experiment: 1-day `VITRIOL_CACHE_KEEP=1` +
  re-prefill log attribution before any `cache_ram` change (states
  2.5–7 GiB vs cache_ram 8192 MiB — alloc-skip skips the Optane spill).
- Runtime `/resume` onto a peer-held session file is still possible
  (startup fence only); the registry gives the operator visibility.
- Legacy `routing-registry.json` cleanup after one transition week.

## Gates

- Extension work: `npm test` in `officina/` + `node runtime/build-patch.mjs`
  canaries when the build surface is touched.
- TUI work: `cargo test` in `officina/tui/` + `cargo build --release`
  + `cp target/release/officina ~/.local/bin/officina` (feature is DONE
  only when the installed binary shows it).
- Commit after every stage (AGENTS.md workflow rule).
