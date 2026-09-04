# Automatic session resume — persistence chain unblocked — 2026-09-04

Owner request: "if the system is killed and restarted, just resume the
session automatically. It's a resilient property either way."

## Root cause (established during the OOM investigation)

The sidecar (`scripts/lull_slot_persist.py`) ALREADY implements the full
resilience chain — startup restore, 300s autosave with activity
signature, empty-slot guard, hang watchdog, oom-shield, and the
**proactive bounce** (checkpoint + clean restart BEFORE a wedge). It was
entirely dead: the engine runs `--mmproj` (multimodal), and the lull
engine gates slot save/restore with
`501 "This feature is not supported by multimodal"`.

The AGENTS.md Stage-6 note attributed the 501s to missing endpoints —
wrong. The endpoints exist (lull server-context.cpp post_slots, gated
only on `params.slot_save_path`); the multimodal mode is what 501s them
(mtmd image chunks can't round-trip the token-sequence state file).

Evidence: `curl POST /slots/0?action=restore` with mmproj loaded →
`{"code":501,"message":"This feature is not supported by multimodal"}`.
After dropping mmproj → same call behaves (400/500 for stale/absent
files, real save works).

## Fix

1. **Drop `model.mmproj`** from the live config (owner: no image input).
   Fingerprint loses `vis=on` (flagged as a delta by the journal —
   working as designed); re-blessed. ~800MB VRAM freed. Vision
   re-enables via `model.mmproj` any time (persistence shuts off again).
2. **Sidecar capability probe**: tri-state PERSIST_OK; on first 501 log
   once ("slot persistence UNAVAILABLE — cold restarts only") then skip
   save/restore silently (no more per-tick 501 spam). Health watchdog +
   proactive bounce stay active regardless.

## Verification

- Save path: `saved slot0.bin: 168 tokens (159995376 bytes)` — fresh
  checkpoint, no 501.
- Restore path: **3/3 consecutive SIGKILL cycles** of the engine →
  auto-restart → sidecar generation-change detection → `restored
  slot0.bin: 168 tokens (~300ms)` each time. Warm KV reload at the
  engine level is dependable.
- Units: vitriol-server + vitriol-autosave both active; engine healthy;
  fingerprint matches blessed.

## Note (not yet measured)

Prefix-match efficiency for pi's next full-history request depends on
pi's `cache_prompt` behavior + the `--cache-idle-slots` host-RAM eviction
path — a real-session measurement, not a synthetic one. Synthetic tests
were abandoned (tokenizer-exactness trap + engine-load timing races).
The engine-level warm restore is proven; client-level cache reuse will
be visible in the owner's next live session after an engine bounce.

## Housekeeping

- Old Sep-2 session-turn checkpoints (~2G) pruned after verification
  (owner-approved); `slot0.bin` kept (now current, 160MB).