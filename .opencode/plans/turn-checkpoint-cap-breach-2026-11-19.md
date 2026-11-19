# Cap breach root cause: per-turn checkpoints bypass the 40 GiB budget — 2026-11-19

## You were right

The cache DID exceed 40 GB, repeatedly. The engine's own spill cache respects
the cap perfectly — but the Officina per-turn checkpoint feature writes into
the same directory **outside the budget entirely**.

## The mechanism

1. **vitriol-checkpoint** (pi extension) saves a FULL slot state after every
   turn via `POST /slots/<id>?action=save`, filename
   `<session-stem>-turn-N.bin` in the engine's `slot_save_path` (kvtier).
   Contract: `_shared/turnkeys.ts :: turnFilename(turn, stem)`.
2. **The engine's budget scan never sees these files.**
   `server-task.cpp:2394-2398` (`server_prompt_cache_disk_scan`) counts only:
   - `cache-*.bin` — spill states
   - `slot*.bin` — slot snapshots (newest protected)
   A file named
   `2026-11-14T13-45-57-...jsonl.patched-...-turn-110.bin` matches **neither
   prefix**, so it is invisible to `cache_max_gb`, invisible to trim, invisible
   to the TUI panel's dir scan (same name filters).
3. **Math**: one slot state for this model ≈ 2.5 GiB. The briev-lang session
   reached turn 110 ⇒ ~100 × 2.5 GiB ≈ 250 GiB from ONE session. The officina
   session added more. That is the 185 GB you saw.

The engine's own cache-*.bin spills were model-citizens the whole time:
17 evictions, four trim sweeps today (`41.5 GiB → 36.0 GiB` pull-backs).

## Why the TUI panel didn't scream

`tui/src/tui/cache.rs :: scan()` lists every file in the dir (it DOES see the
turn files) — but the budget line computed against them is meaningless once
the engine ignores them, and before today's wipe the panel would have shown
the inflated total. The wipe (`cache_wipe(None)`) deleted every non-slot0
file — including turn checkpoints — which is why `/rewind` KV pairing is now
broken for old turns (snapshot refs survive in git, but the KV half is gone;
`/rewind` will fall back to text-only restore for those turns).

## Fix plan (code mode, staged)

### Stage 1 — stop the leak (extension, no engine rebuild)
`vitriol-checkpoint` retention: after each successful save, keep only the last
K turn files for that session (K = 3 default, `OFFICINA_CKPT_KEEP` knob in
`_shared/config.ts`). Deletes the file via the engine's own
`POST /slots/...`? — no, deletion is plain unlink of the previous turn file
(same-dir, name known). Test: unit test in vitriol-checkpoint that N saves
leave exactly K turn files + slot0.

### Stage 2 — make the budget honest (engine, next rebuild)
Two options, pick one:
- **A (naming)**: `turnFilename` → `slot-turn-<stem>-<n>.bin` so files land in
  the snapshot class. WRONG for the cap: snapshots are protected (newest) and
  per-turn files would shield each other from eviction. Rejected.
- **B (scan, chosen)**: extend `server_prompt_cache_disk_scan` to also count
  `*-turn-*.bin` as budget-visible states (not protected, evictable LRU).
  Invariant I1: files that don't match any known pattern still excluded
  (operator's own files in the dir untouched). Golden test: dir with
  cache-*, slot0, slot2, turn files → cap math includes turns, wipe semantics
  unchanged.

### Stage 3 — regression pins
- Engine unit test: turn file counted in budget; trim evicts oldest turn file.
- Extension test: K-retention; rename/migration for legacy stems.
- Boundary: `turnkeys.ts` is the single naming source — both sides must import
  it (already true for the extension; the engine has its own pattern list, so
  the test must pin the pattern list against turnkeys' output).

### Stage 4 — migration
Existing turn files: one-time cleanup already done by Shift+W (8 files remain
= the two live sessions' recent turns). Keep-last-K will naturally prune them
on the next saves. Optionally a one-liner in the TUI cache panel marking
unknown-pattern *.bin as "unmanaged (not in budget)".

## Invariants
- I1: unknown-pattern files in kvtier are never engine-deleted (parity).
- I2: engine spill cache behaviour byte-identical (cap math unchanged for
  cache-*/slot*).
- I3: `/rewind N` keeps working for the last K turns (KV+snapshot pairing).
- I4: after Stage 1, kvtier cannot grow unboundedly from checkpoint churn:
  ≤ K × 2.5 GiB per live session + spills ≤ 40 GiB + slot0.

## Status
PLAN only — no code touched this turn. Root cause + fix design recorded.
