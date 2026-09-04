# Model-sync hardening — 2026-09-04

Owner-reported from the ontic sessions: persistent
`model mismatch: engine loaded Lapis Occultus, pi selected
Qwen3.8-27B-Q3_K_M.gguf` + connection retries + "can't find Vitriol".
Preceded by the stale-session diagnosis (three officina processes
carrying pre-fix extensions; see session log). After the owner's
restart, the mismatch CHANGED SHAPE (9B name → 27B basename) and still
persists with current code — which turns it from stale-session
archaeology into two real bugs.

## Bug A — the honest mismatch rule has a hole (session-panel)

The rule (ad1954f) accepts pi's string matching:
- the alias (`Lapis Occultus`) ✓
- the STRIPPED basename (`Qwen3.8-27B-Q3_K_M`, `.gguf` removed) ✓
- the full path (`/home/.../Qwen3.8-27B-Q3_K_M.gguf`) ✓

pi's natural model id is the GGUF FILENAME WITH extension
(`Qwen3.8-27B-Q3_K_M.gguf`) — matches none of the three → permanent
false mismatch for a perfectly aligned session.

**Fix**: extract `modelMatchesLoaded()` and add the fourth shape:
filename-with-extension (`loaded_path.split("/").pop()`).

## Bug B — the auto-sync is one-shot and fragile (llama-cpp-provider)

The session_start sync calls `setModel` once. Evidence (pi still holds
the basename despite every opportunity to sync) says the sync can
silently no-op: `ctx.modelRegistry` absent at session_start, or the
engine unreachable at spawn (today's 12:06–13:13 maintenance windows —
exactly when the ontic sessions restarted; the `retry #1` flapping was
the session hitting the engine during unit bounces).

**Fix**: bounded retry — attempt fetch + setModel every 5s up to 12
times, clear the timer on first success; re-arm on every session_start
(resume paths included). A session that spawns into an engine-down
window now self-heals the moment the engine returns.

## Also

- Ancient officina process (PID 3487076, up from Sep 2 17:33, carrying
  pre-everything extensions) killed after the fixes land.
- Follow-up candidate (not this batch): stamp carried extensions with a
  source-version hash so stale sessions warn at spawn.

## Verification

vitest: all four match shapes + true-mismatch negative. tsc clean.
Restart of the ontic TUIs (owner) picks up the fixed code; expected
end-state: clean loaded row, no mismatch notify, header shows
`Lapis Occultus`.
