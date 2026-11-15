# Officina harness self-healing

- Date: 2026-11-15
- Status: implemented 2026-11-15 (rebuilt + installed)
- Origin: operator report — the TUI surfaces
  `harness watchdog: ... cancelling and aborting the turn` and
  `abort did not settle after 10s ... /quit then relaunch Officina`.
  A harness that can recover the child, the session and the environment
  must do so itself instead of asking the operator to restart the UI.

## Goal

Turn the current one-shot watchdog plus manual restart into a bounded,
observable recovery ladder:

1. **Abort** — cancel extension dialogs, send `abort`, wait 10 s.
2. **Respawn** — if abort does not settle, or the pi child exits:
   kill the whole pi process tree, append-only repair the active session
   if needed, respawn pi on the same session file, rehydrate transcript
   and widgets.
3. **Environment heal** — on model/engine failures:
   - engine down and the selected model is **`officina/lapis`**:
     start `vitriol-server.service` if inactive;
   - selected model unavailable/not found: switch to `officina/lapis`;
   - auth / rate-limit / context-overflow: typed notice only (the routing
     and compaction layers already own those).
4. **Replay** — resend the failed prompt once, only when the failed turn
   ran zero tools and produced zero assistant output. Slash commands and
   image-bearing prompts are never replayed.
5. **Degrade safely** — after the attempt budget, keep the TUI alive with
   input preserved and an explicit `/recover` command.

## Hard rule: engine autostart is lapis-only

The TUI may start `vitriol-server.service` **only when the selected model
is `officina/lapis`** (provider `officina`, id `lapis`). For every other
selection — a physical `llamacpp/*` alias, Caduceus, Nimbus, any cloud
model — an engine-down error is surfaced but never auto-started. There is
no "start it anyway" fallback.

`/engine start` remains available as an explicit operator action.

## Current failure surfaces

- Tool-stall watchdog: `officina/tui/src/tui/mod.rs:2590`.
- Abort-settle deadline releases the UI and prints the manual restart hint.
- Reader channel `None` prints “agent process exited” and breaks the TUI
  event loop (`mod.rs:699`).
- Model errors are surfaced as chat text only (`mod.rs:3341`).
- `RpcBridge` owns exactly one child; `run_loop` owns exactly one reader
  channel, so no in-process respawn path exists.

## Stages

### 1. Respawnable bridge

Files: `rpc/bridge.rs`, `rpc/protocol.rs`, `tui/mod.rs`

- Store `PiLaunch { cli_path, cwd, extra_args }` in `RpcBridge`.
- Spawn pi in its own process group (`Command::process_group(0)`) so a
  stuck tool subtree can be terminated as one unit.
- `terminate_tree()`: SIGTERM the group, wait ~2 s, SIGKILL the group,
  reap the child. Fall back to `Child::kill` when the group is gone.
- `respawn(session)`:
  - terminate the old tree;
  - filter `--session`, `--fork`, `--session-id` from the saved args;
  - add `--session <path>` when a session file is known;
  - reuse the existing extension carry/trust logic and diag ring;
  - return a fresh reader channel and swap the bridge fields.
- Parse `sessionFile` from `get_state` into `RpcSessionState` and
  `AppState.session_file`.
- Change `run_loop` to hold `&mut UnboundedReceiver<WireMessage>` so a
  respawn can install the new reader.

### 2. Recovery policy and state

Files: `tui/heal.rs` (new), `tui/state.rs`

- `heal.rs`:
  - `Failure` classification (engine down, no physical model, model not
    found, auth, rate limit, context overflow, unknown);
  - `Policy` (default: 3 attempts / 10 min, backoff 1 s, 5 s, 15 s);
  - `RecoveryState` (phase, attempts, reason, degraded, pending replay,
    engine deadline, rehydrate deadline);
  - `is_lapis(model)` gate for engine actions.
- `AppState`:
  - `session_file: Option<PathBuf>`;
  - `last_wire_at` + `note_wire_activity()` on every wire message;
  - `last_prompt: Option<LastPrompt>` set when a real prompt is sent;
  - `recovery: RecoveryState`.
- `LastPrompt` records text, cloned images, `tool_started`,
  `assistant_output`, `failed`, `replayed`.
  `tool_execution_start` sets `tool_started`; assistant deltas set
  `assistant_output`; a failed `message_end` sets `failed`; a clean turn
  clears the record.

### 3. Wire-idle detector

Files: `tui/mod.rs`, `tui/state.rs`

- Keep the tool watchdog.
- Add wire-idle: streaming/compacting, no running/expected tool, no wire
  message for `OFFICINA_WIRE_IDLE_SECS` (default 300).
- Before escalating, gather liveness evidence:
  - pi subtree CPU delta;
  - engine `/slots` probe (`is_processing` / active prefill).
  If the engine is busy, extend the deadline instead of aborting a long
  prefill.
- Abort-deadline expiry escalates to respawn instead of only releasing
  the UI.
- Reader `None` triggers recovery instead of breaking the loop.

### 4. Model / engine actions and replay

Files: `tui/mod.rs`

- `message_end` with `stopReason == "error"` feeds `classify_error`.
- Engine down / no physical model:
  - start the engine **only when `is_lapis(state.model)`** and
    `systemctl is-active` says inactive;
  - wait for engine readiness up to 90 s;
  - never restart an already-active engine unless
    `OFFICINA_HEAL_ENGINE_RESTART=1`.
- Model not found → `SetModel officina/lapis` once, unless already lapis.
- Deduplicate by failure class + 60 s cooldown.
- Replay gate: failed prompt, no tools, no assistant output, no images,
  not a slash command, not already replayed. One replay per failure.
- Post-rehydration safety: if the transcript shows a tool call after the
  last user message, do not replay.
- `/recover` (alias `/heal`): manual recovery attempt; works in the
  degraded state.

### 5. UI, diagnostics, install

- Notices name the recovery phase and attempt count; `/diag` keeps the
  recovery trail.
- F1 help documents the environment knobs.
- Rebuild and install: `cargo build --release` in `officina/tui/`, then
  `cp target/release/officina ~/.local/bin/officina` (AGENTS rule: a TUI
  change is not done until installed).

## Environment knobs

| variable | default | meaning |
|---|---|---|
| `OFFICINA_SELF_HEAL` | `1` | master kill switch; `0` restores today's behavior |
| `OFFICINA_WIRE_IDLE_SECS` | `300` | no-wire hang threshold; `0` disables |
| `OFFICINA_HEAL_MAX_ATTEMPTS` | `3` | attempts inside the window |
| `OFFICINA_HEAL_WINDOW_SECS` | `600` | attempt window |
| `OFFICINA_HEAL_REPLAY` | `1` | `0` disables automatic replay |
| `OFFICINA_HEAL_ENGINE` | `1` | allow engine autostart |
| `OFFICINA_HEAL_ENGINE_RESTART` | `0` | allow restarting an active engine |
| `OFFICINA_ENGINE_URL` | `http://127.0.0.1:8279` | engine probe endpoint |

## Invariants

- **I1 parity** — with `OFFICINA_SELF_HEAL=0`, behavior is byte-identical
  to today: no respawn, no replay, existing watchdog messages unchanged.
- **I2 session integrity** — never repair or reopen a session while the
  old child may still write; terminate the tree, wait, use append-only
  repair, then respawn.
- **I3 safe replay** — tool-touched turns are never auto-replayed; at most
  one replay per failure; slash commands and image prompts are excluded.
- **I4 bounded recovery** — max attempts + backoff; exhaustion leaves the
  TUI alive with `/recover`.
- **I5 lapis-only engine autostart** — no selected model other than
  `officina/lapis` may trigger `systemctl start|restart`.
- **I6 abort safety** — the existing 10 s abort-settle release still
  happens before any respawn.
- **I7 provenance** — every recovery notice carries reason + attempt count.

## Tests and gates

- `cargo test` and `cargo fmt --check` in `officina/tui`.
- Unit tests:
  - failure classification table;
  - policy backoff/exhaustion with injected time;
  - `sessionFile` parsing;
  - restart argument filtering;
  - replay gate matrix (tool / assistant output / images / slash);
  - wire-idle decision matrix (recent wire, CPU active, engine busy,
    all flat);
  - lapis-only engine gate.
- Manual drills:
  1. `kill -STOP` pi mid-turn → abort deadline → tree kill → respawn →
     transcript restored.
  2. `kill -9` pi → auto-respawn, TUI stays up.
  3. Stop engine with lapis selected → engine starts, one tool-free
     prompt replay.
  4. Stop engine with a physical/cloud model selected → error surfaced,
     engine **not** started.
  5. `sleep 500` bash + `kill -STOP` pi → tool cancelled, no replay.
- Rebuild + install; verify the notice appears in the installed binary.

## Rollback

`OFFICINA_SELF_HEAL=0` restores the old path without touching config,
profiles or the engine.

## Implementation notes (as landed)

- Plan written before code changes.
- `rpc/bridge.rs`: `PiLaunch`, per-child process group, `terminate_tree`,
  `respawn`, `restart_args`; real-node respawn test passes.
- `rpc/protocol.rs` + `tui/state.rs`: `sessionFile` captured from
  `get_state`; `LastPrompt` replay gate; `RecoveryState`; wire-idle and
  rehydrate-reset helpers.
- `tui/heal.rs`: failure classification, bounded policy/backoff,
  lapis-only gate (`is_lapis`), engine `/slots` liveness probe.
- `tui/mod.rs`: abort-unsettled → respawn; child-exit recovery;
  wire-idle detector with pi-CPU + engine-prefill grace; provider-failure
  actions; one-shot replay; `/recover` / `/heal`.
- `session_guard::repair_for_resume`: validate → append-only patch →
  conservative repair; never silently quarantines an active session.
- Gates: `cargo test` 273 passed (268 binary + 5 protocol), `cargo fmt --check` clean, `cargo clippy -- -D warnings` clean; release
  build/install performed with an atomic rename (the old binary was in
  use by live TUI processes). New bridge tests exercise a real node
  respawn and verify terminate_tree reaps a grandchild `sleep`.
- Engine autostart is gated on `officina/lapis` in `handle_provider_failure`;
  every other model produces an explicit "auto-start skipped" notice.
