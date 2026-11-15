# Sessions "hanging on the watchdog" — diagnosis + plan

- Date: 2026-11-15 (investigation window 21:24–22:40 local / 20:24–21:40 UTC)
- Status: **stages 0–3 implemented, gates green, installed** (details in §9).
  The blocker's identity still needs one live reproduction with the new trail (§10).
- Mode: PLAN for the investigation; GRIND for the implementation.
- Trigger: operator report — "several sessions seem to be hanging on the watchdog",
  then "the harness keeps resetting the session", then "another auto recover".
  The investigation session itself reproduced the failure four times.

## 1. TL;DR

Two different failures are being conflated under one operator-visible symptom:

1. **The destructive loop (proven).** A `bash` tool call is started by pi, then produces
   **zero bytes and never exits**. After `OFFICINA_TOOL_WATCHDOG_SECS` (default **300 s**)
   the TUI aborts the turn; pi's abort cannot cancel a tool that is stuck *before* it
   executes, so 10 s later the self-heal respawns pi (`terminate_tree` → `--session`
   resume → transcript rehydrate). `session_guard` then appends a synthetic
   `[interrupted] Tool call was interrupted before execution and was not run.` result, so
   the turn is silently discarded. Next `bash` call: same thing. Measured cycle:
   **300 s + 10 s = 310 s**, exactly, four times in this session.
2. **The watchdog is advice-only (by design) and reads UNKNOWN (bug-shaped).**
   `hang-watchdog` never kills; it writes facts + a chat card. Its judge returns
   `UNKNOWN — judge reply unparseable` almost always, so the card cannot distinguish
   "working" from "wedged". A truly wedged tool therefore sits on screen for **43 minutes**
   (live case: session `ea51662b`, `elapsed_s = 2582`, `stalled_s = 2582`, phase `monitor`)
   with no operator-actionable information.

Net: the TUI's reaction (kill + restart) is the *only* recovery, and it is triggered by a
300 s silence budget that the model cannot see or extend, so the operator experiences
"the harness keeps resetting the session" while the real fault (pi never spawning the
shell) is never surfaced.

## 2. Proven mechanism, with code

`officina/tui/src/tui/mod.rs` (uncommitted self-heal tree, installed binary
`~/.local/bin/officina` md5 `363920b1…`, built 20:40, matches `tui/target/release`):

| step | code | effect |
|---|---|---|
| `tool_execution_start` → `state.tool_start()` | `mod.rs:3889`, `state.rs:2858` | `ChatEntry::Tool{running:true, started_at}` armed |
| 300 s later, `oldest_running_tool()` age ≥ limit | `mod.rs:2898-2911` | Warn: "`{name}` idle 300s with no result — cancelling and aborting the turn" |
| abort sent, `abort_deadline = now+10s` | `mod.rs:2916-2924` | `RpcCommand::Abort` |
| pi emits no `agent_end` (it is stuck *before* execution, so there is nothing to abort) | — | deadline expires |
| `maybe_heal` respawn path | `mod.rs:2666-2700`, `recover()` 2929+ | "abort did not settle after 10s — restarting pi; session preserved" → `RpcBridge::respawn()` (`rpc/bridge.rs:423`) |
| respawn pins `--session <file>`, rehydrates, and the guard patches the dangling call | `bridge.rs:132-159`, `session_guard.rs:495-620` | append-only synthetic `[interrupted] …` tool result |

Measured, from `~/.pi/agent/sessions/--home-randozart-Desktop-Projects-VITRIOL--/`
(patch timestamps are in the file name; gaps are exact):

| assistant msg with tool call (UTC) | repair patch (UTC) | gap | note |
|---|---|---|---|
| 20:27:44.647 | 20:32:54.853 | **310 s** | 2 parallel bash calls, both never ran |
| 20:33:38.254 | 20:38:48.941 | **310 s** | 2 parallel bash calls |
| 20:42:25.236 | 20:47:35.325 | **310 s** | 1 bash call |
| 21:51:07 (derived) | 21:56:17.626 | **310 s** | deliberate probe (below) |

Repair patches in this one session: **4** (`…patched-1794774774853`, `…-1794775128941`,
`…-1794775655325`, `…-1794776577626`). Session started 21:24 local.

### 2.1 The stall is real and total: the command never produces a byte

Live fact file `~/.vitriol/officina/watchdog/01a1e0e1-….json`, written by `hang-watchdog`
for my own deliberately trivial probe:

```json
{"command":"echo PROBE-START; ps -p 1103839 -o pid,stat=,wchan:16= 2>&1 | head -3; timeout 5 ls -l /proc/1103839/fd 2>/dev/null | grep -c . ; echo PROBE-END rc=$?",
 "elapsed_s":208,"stalled_s":208,"kind":"unavailable","verdict":"UNKNOWN",
 "reason":"judge reply unparseable — liveness unknown","phase":"rejudge"}
```

`stalled_s == elapsed_s` ⇒ **`echo PROBE-START` never arrived**. The command is framed by
`timeout 5`, so the shell itself cannot hang either. The tool was therefore never spawned
(or its spawn was never dispatched), even though pi emitted `tool_execution_start`
(`hang-watchdog` keys `startedAt` off it).

Same signature in the other live session (`ea51662b`): `elapsed_s = stalled_s = 2582` for
`grep "^rchar:" /proc/3771111/io …` — no live bash child of that pi exists, and the pi is
still waiting 43 min later. By contrast, every command that *did* run reports
`stalled_s = 0` and `reason: "new output from this tool"` (e.g. `01a1cbb2` census at
`elapsed_s 900`, `c8a0e48c` `cargo test`, `officina-…` brievc build).

### 2.2 pi's event loop is alive during the stall

`hang-watchdog` published its 60 s notice and completed a 180 s judge attempt (the fact
file exists with `kind: unavailable`, `elapsed_s: 208`), so pi is *not* frozen; something
between `tool_execution_start` and `child_process.spawn()` is pending and un-cancellable.

Per pi docs (`docs/extensions.md:971-985`): `tool_execution_start` fires, **then**
`tool_call` handlers run ("can block"), then the tool executes. Before `tool_call`, pi
"waits for previously emitted Agent events to finish draining through `AgentSession`".
A handler that never resolves therefore reproduces every symptom:
no spawn, no output, no exit, no abort possible, and a live event loop.

### 2.3 Candidate blockers (narrowed, not yet identified)

| candidate | evidence for | discriminating test |
|---|---|---|
| a pending `tool_call` handler | only `permissions-guard` awaits here (`index.ts:113-190`); mode file says `yolo`, so it should short-circuit to allow | log `permissions-guard` verdict+timing per bash call |
| an unfinalised Agent event handler (the drain gate) | `message_end`/`agent_end` handlers that await model/engine work: `composer-predictions:224`, `vitriol-checkpoint:139` (`turn_end` → `slotOp("save")` against a **single-slot engine** shared with 4 other sessions), `plan-mode:788` (`ctx.ui.select` on plan approval) | timestamp each hook's entry/exit; the 300 s one is the culprit |
| an extension UI request the TUI ignores | `handle_ui_request` `_ => {}` (`mod.rs:4245`) — **unknown methods get no reply**, so pi waits forever; `select` with empty options auto-cancels, everything else queues a modal that is never re-shown after `reset_for_rehydrate()` clears `state.dialogs` | log every `extension_ui_request` method + id + reply; answer unknown methods with `cancelled:true` |
| engine-queue blocking | 4–5 sessions share `vitriol-server --parallel 1`; any extension `fetch` to it without a timeout queues behind another session's generation (`_shared/engine.ts:198`) | engine `/slots` probe at stall time (already implemented for the wire-idle path, *not* for tool stalls) |

Ruled out: `diagnostics-loop` praetor/syntax checks (edit/write only), `core-rpc` sidecar
(250 ms timeout, fail-open, `OFFICINA_ROUTER_CORE` gate), `plan-mode`/`tool-gating` blocks
(bash is not gated), `longjob` auto-dispatch (my commands are not long-runners), model
failure (`message_end` `stopReason` never `error`), OOM (no kills, engine healthy,
`vitriol-server` up 13 894 s), disk/CPU starvation (short commands succeed minutes apart).

## 3. Secondary defects found while reading

1. **`UNKNOWN` is the steady state of the judge.** `parseVerdict` (`hang-watchdog/judge.ts`)
   rejects its own prompt format: it requires exactly one `VERDICT:` line and rejects the
   reply if the evidence line merely *mentions* `ALIVE|HUNG|UNKNOWN`. Every card I
   observed ends `judge reply unparseable — liveness unknown`. The judge prompt asks for
   "one short sentence of evidence", so the parser is fighting its own prompt. Result: the
   operator can never distinguish a 6-minute compile from a wedged shell.
2. **The watchdog cannot help by design.** `index.ts:1-2` "this extension never aborts";
   `MAX_JUDGMENTS_PER_CALL = 3` then `phase: monitor` forever. Combined with (1), cards rot
   on screen (`valid_until` keeps being pushed every 30 s tick → never expires).
3. **`SESSION_LOG` truth gap.** Nothing on disk records *why* a stall happened (the TUI
   diag ring is memory-only; `last-crash.log` does not exist; `OFFICINA_TRACE_EVENTS` is
   off). Post-mortems like this one have to be reconstructed from repair patch names.
4. **Sessions whose tools stall are *lost*, not replayed.** `take_replayable_prompt()`
   refuses when `tool_started` is true, and `tool_start()` sets it — so a respawn never
   re-sends the prompt; the operator's turn simply vanishes (only the synthetic
   `[interrupted]` remains).
5. **The 300 s budget is invisible and non-overridable from the model side.** It is honoured
   only if the model passes a `timeout` argument (`mod.rs:2901-2904`: `max(300, timeout+60)`).
   A long silent build launched without `timeout` is killed at 300 s — the same abort →
   respawn path — i.e. *legitimate* work is indistinguishable from a wedge.
6. **`terminate_tree` does not reach tool subtrees.** pi's bash tool spawns with
   `detached: process.platform !== "win32"` (`pi dist/core/tools/bash.js:63`), i.e. the shell
   is `setsid`ed out of pi's process group, so the respawn's `signal_group(pid, "TERM")`
   (`bridge.rs:402-417`) misses running tools. Orphaned subtrees are expected.

## 4. Plan

Ordering is deliberate: **instrument → stop the thrash → fix the blocker → fix the truth
surface.** Stage 0 is mandatory before stage 2; without it we would be guessing which
handler blocks.

### Stage 0 — make a stall self-explanatory (cheap, no behaviour change)

1. `officina/tui/src/tui/crashlog.rs`: add `stall_trail(event)` → append one JSON line to
   `~/.vitriol/officina/stalls/trail-<pid>.jsonl` (bounded, 0600, same style as
   `capture_event`). Record: `tool_execution_start`, each `extension_ui_request`
   (method + id + reply/cancelled), `abort sent`, `abort settled|not`, `respawn`,
   `rehydrate done`, and the engine probe result at abort time.
2. `handle_ui_request`: **answer unknown methods with `{"cancelled":true}`** instead of
   `_ => {}` (defect 3.1 above; also removes a whole class of indefinite waits).
3. Extension-side hook timing: a tiny `_shared/hook-trace.ts` that wraps every
   `pi.on(...)` registration, logs `enter/exit` with wall time when
   `OFFICINA_HOOK_TRACE=1`. One-line opt-in; zero cost when off.
4. Reproduce once (a bash call whose command is `sleep 400 #fg`), collect the trail, and
   name the blocking hook. **Gate for stage 2.**

### Stage 1 — stop the destructive loop (works even before the root cause is fixed)

1. **Never respawn on a tool stall first touch.** Distinguish
   *tool started but silent* (our case) from *abort did not settle*. For the silent case:
   before aborting, gather evidence exactly like the wire-idle path already does —
   pi-tree CPU delta, engine `/slots` (`is_processing`), and whether a shell process for
   that tool exists under the pi tree. Only abort on flat evidence; otherwise
   `grant_grace(120 s)` and say so in the transcript.
2. **Escalate, don't restart, on the first stall.** Ladder: warn+grace → abort →
   (only if the *same* tool id stalls twice) respawn. Bounded by the existing
   `heal::Policy` budget so a genuine wedge still recovers.
3. **Keep the turn.** If `tool_started` but the tool never produced output and never
   executed (the stall signature), the turn is replayable: relax
   `take_replayable_prompt()` to allow `tool_started` when
   `abandoned_stall && !produced_output && !assistant_after_tool`. At most one replay,
   same I3 gate.
4. **Model-visible budget.** Surface the silence budget in the bash tool description
   (`bash-title` already re-registers `bash`): "commands silent for >300 s may be aborted;
   pass `timeout` for long builds". Optionally auto-derive `timeout` from a
   `#fg`-style annotation already used by `permissions-guard`.
5. Config: `OFFICINA_TOOL_WATCHDOG_SECS` stays 300 but `OFFICINA_STALL_GRACE_SECS`
   (default 120) and `OFFICINA_STALL_RESPAWN_AFTER` (default 2) are added; both ride the
   verdict, not the data path.

### Stage 2 — fix the actual blocker

Whichever stage 0 names:

- *pending `tool_call` handler* → add an abort-aware timeout wrapper around the handler
  (block + reason "permissions veto timed out") so a stuck guard fails **closed but
  fast** instead of hanging the turn.
- *pending Agent-event handler (drain gate)* → move the await off the drain path: make
  `vitriol-checkpoint`'s `turn_end` save fire-and-forget with a bounded internal timeout
  (it must never gate the next turn), and give `composer-predictions` a hard deadline.
- *engine-queue blocking* → every extension `fetch` to the engine gets
  `AbortSignal.timeout(N)` and a queued-request fast path.
- *UI request* → already covered by stage 0.2.

### Stage 3 — truthful watchdog surface

1. `parseVerdict`: accept the single `VERDICT:` line unless the evidence rest *re-declares*
   a verdict; treat `unparseable` distinctly from `UNKNOWN` (render `NO-JUDGE`), so
   "judge broken" stops masquerading as "liveness unknown".
2. Prompt: forbid the words ALIVE/HUNG/UNKNOWN in the evidence sentence.
3. `phase: monitor` must stop pushing `valid_until` forever: after the budget, publish one
   terminal card (`MONITORING`) and let it expire; the TUI then drops a stale card instead
   of showing a permanent amber box (defect 3.2; relates to the
   `color-semantics-2026-11-14` card contract).
4. Card text: when `stalled_s == elapsed_s`, say **"no output at all — command may not have
   started"**, which is the operator's actionable signal.

## 5. Invariants

- **I1 parity**: with `OFFICINA_SELF_HEAL=0` behaviour is unchanged.
- **I2 no silent turn loss**: a stalled turn is either replayed once (bounded) or surfaced as
  an explicit error entry naming the command; never discarded.
- **I3 replay safety**: replay only when the turn produced no tool *execution*, no assistant
  text, no images, not a slash command; at most once per failure.
- **I4 bounded recovery**: attempt budget + backoff unchanged; exhaustion leaves the TUI
  usable with `/recover`.
- **I5 no new destructive actions**: stage 1 adds *evidence gathering before* abort, never a
  new kill path.
- **I6 facts truth**: a card never claims `ALIVE`/`UNKNOWN` on behalf of an unparsed judge.
- **I7 provenance**: every recovery notice carries reason, attempt count, and the trail file
  path.

## 6. Gates

- `cargo test` + `cargo fmt --check` in `officina/tui` (new tests: stall-trail shape; unknown
  UI method answered `cancelled`; grace-before-abort matrix; replay-on-stall matrix;
  `parseVerdict` NO-JUDGE cases).
- `node --test` for `officina/.pi/extensions/hang-watchdog/*.test.ts` (judge prompt/parse,
  terminal card expiry).
- Rebuild + reinstall (`cp target/release/officina ~/.local/bin/officina`) — a TUI change is
  not done until installed (AGENTS.md).
- Live drill: one deliberate `sleep 400 #fg` call → trail names the blocker, no respawn on
  first touch, turn replayed exactly once.
- Report `.opencode/plans/` with the trail excerpt and the fingerprint of the installed build.

## 7. Operator mitigations available right now (no code)

- `OFFICINA_SELF_HEAL=0` in `~/.vitriol/officina/env` → the old behaviour (release the UI,
  keep pi), so no more silent session restarts; a wedged pi is then restarted by hand.
- Pass `timeout` in long bash commands, or prefix `#fg`-class annotations the
  `permissions-guard` long-runner dispatch already understands, so the 300 s floor lifts to
  `timeout + 60`.
- `OFFICINA_NO_HANG_WATCHDOG=1` silences the advisory cards (they are currently all
  `UNKNOWN`, i.e. noise) while the real fix lands.

## 8. Decisions (locked 2026-11-15, agent-owned)
1. **First-touch stall = grace, not abort.** A silent running tool gets evidence gathering
   (engine `/slots`, pi-tree CPU delta, shell process present under the pi tree) and, if the
   evidence is flat, one bounded grace of `OFFICINA_STALL_GRACE_SECS` (default 120 s). Abort
   only after the grace expires for the *same* tool call. Two stalls of the same call ⇒
   abort, and the existing self-heal ladder continues from there.
2. **No new engine autostart.** Tool stalls never start or restart `vitriol-server`; the
   existing lapis-only autostart stays exactly where it is (wire-idle path). A stall on a
   down engine is reported, never acted on.
3. **Bounded stall replay (I3 relaxation, narrow).** A turn whose only tool never executed
   (started, zero output, abandoned by the stall path) is replayable **once**, provided the
   prompt is not a slash command, carries no images, and the transcript has no *successful*
   tool after the last user message. Everything else keeps today's stricter gate.
4. Stage 0 instrumentation ships unconditionally (bounded files), not behind an opt-in —
   the failure it exists to explain is rare and unreproducible on demand.

## 9. Implementation notes (as landed 2026-11-15, 23:05)

Installed binary: `~/.local/bin/officina` md5 `8b07965b28ef8f5ea96aa45741d0a8c7`
(previous live build `363920b1…`). Running sessions keep the old image in memory;
the hardening takes effect on the next launch.

### New module — `officina/tui/src/tui/stall.rs`

- `trail(event, fields)` → `~/.vitriol/officina/stalls/trail-<pid>.jsonl`
  (`OFFICINA_STALL_TRAIL_DIR` overrides the root). Bounded at 2 MiB, 0600,
  never fails a turn. Unconditional (decision 4).
- `Evidence { pi_flat, engine_busy, descendants }` and
  `stall_verdict(silent, single_running_tool, grace_used, ev)`:
  `NeverStarted` → `Working` → `Grace` → `Abort`, in that precedence.
- `UNSTARTED_SECS = 60`: the decisive fast path. A silent, lone tool with
  **no descendant process** can never start, so it is recovered in ~70 s
  instead of ~310 s.

### `officina/tui/src/tui/mod.rs`

- `maybe_heal`'s stall block became `StallCase` → `maybe_grace_or_abort` →
  `evaluate_stall` + `apply_stall_verdict` + `stall_notice` + `abort_for_stall`
  (extracted so `maybe_heal` stops growing; Praetor clean on the new code).
- Evidence before abort: engine `/slots` `is_processing` (only when pi is flat),
  pi-subtree CPU (existing liveness sampler), live descendant count
  (`state::subtree_process_count`). Re-probes are parked (30 s decisive,
  60 s working, `OFFICINA_STALL_GRACE_SECS` on grace) so a 90 ms heartbeat
  cannot hammer the engine endpoint.
- `handle_ui_request`: an unknown method, or a dialog request without an id, is
  now answered `{"cancelled":true}` instead of being dropped — a dropped request
  is an indefinite wait for pi.
- Trail breadcrumbs: `tool.start`, `tool.end`, `stall.evaluate`, `abort.sent`,
  `abort.settled`, `respawn.fail`, `rehydrate.done`, `ui.unknown`,
  `ui.unknown_id`.

### `officina/tui/src/tui/state.rs`

- `RunningTool { key, name, age, timeout_s, silent }` replaces the anonymous
  tuple from `oldest_running_tool()`; `silent` = not one output line arrived.
- `running_tool_count()`, `pi_is_flat()`, `subtree_process_count()`,
  `stall_parked()/park_stall()/clear_stall_state()`.
- `LastPrompt.stalled_tool` + `note_stall_unexecuted()`; the replay gate
  (`last_prompt_replayable`/`take_replayable_prompt`) now owns the side-effect
  veto itself (`replay_has_side_effects`) so no caller can bypass it.
  `transcript_has_successful_tool_after_last_user()` ignores abandoned/errored
  calls.

### `officina/tui/src/tui/heal.rs`

- `Policy.stall_grace` (`OFFICINA_STALL_GRACE_SECS`, default 120).

### Extensions

- **hang-watchdog**: `NO-JUDGE` is a distinct verdict (a broken judge can no
  longer masquerade as `UNKNOWN`); the judge prompt forbids re-declaring a
  verdict in its evidence sentence (that is what made every reply
  "unparseable"), and states that "no output at all, for a command that should
  have printed immediately" is HUNG evidence. Budget exhaustion publishes **one
  terminal** card with `next_check_at: 0`; no further cards, so the TUI's lease
  expiry drops it instead of showing a countdown that never advances.
- **vitriol-checkpoint**: `turn_end` autosave is fire-and-forget with a
  single-flight guard (`pendingAutosaves` is the test seam). The engine is
  single-slot; awaiting a save on pi's drain path could delay the next tool by a
  whole generation.
- **`_shared/engine.ts`**: one `AbortSignal.timeout` covers connect **and** body
  read (the old code cleared the timer before `res.text()`).
- **`_shared/hook-trace.ts`** (new): `OFFICINA_HOOK_TRACE=1` wraps `pi.on` for
  an extension and writes enter/exit/duration to
  `~/.vitriol/officina/hooks/<ext>.jsonl`. Wired into `permissions-guard`,
  `vitriol-checkpoint`, `composer-predictions`, `plan-mode` — the four handlers
  that can sit on pi's drain path before a tool executes.
- **bash-title**: the model now sees the silence budget ("a command silent for
  300 s is aborted: pass `timeout` for long builds").

## 10. Invariants → where they are pinned

| invariant | pinned by |
|---|---|
| I1 parity (`OFFICINA_SELF_HEAL=0`) | `heal::self_heal_enabled()` guards unchanged; `maybe_heal` returns before any stall logic |
| I2 no silent turn loss | `state::tests::stall_replay_needs_a_never_executed_tool_and_happens_once` |
| I3 replay safety | `state::tests::stall_replay_needs_a_never_executed_tool_and_happens_once` + `..._is_blocked_by_a_real_tool_result` |
| I4 bounded recovery | `heal` policy tests (unchanged) + `stall_park_prevents_repeated_evidence_probes` |
| I5 no new destructive action | `stall::tests::{never_started_beats_every_other_arm, live_evidence_grants_time_before_the_grace, flat_evidence_gets_exactly_one_grace_then_aborts}` — every arm that aborts is behind an evidence gate |
| I6 facts truth | `hang-watchdog/judge.test.ts` NO-JUDGE cases + `runtime.test.ts` terminal-card assertions |
| I7 provenance | `stall::tests::trail_appends_parseable_jsonl_and_stops_at_the_cap` |

Gates run: `cargo test` 279 passed, `cargo fmt --check` clean, `npx tsc --noEmit`
clean, `npx vitest --run hang-watchdog vitriol-checkpoint` 98 passed / 1 skipped,
Praetor: zero findings on the new/changed functions. Isolated-PTY smoke test of
the installed binary: launches, quits clean, no panic.

## 11. Next live drill (needs a freshly launched TUI)

1. Launch Officina (picks up `8b07965b…`).
2. Ask the agent for a deliberately silent tool, e.g.
   `bash: sleep 130 #fg` (no `timeout`).
3. Expect in the transcript: `harness: bash silent for 300s with no process
   evidence — holding 120s before any abort (pass timeout for long builds)`,
   then, if it is still silent, an abort and a **one-time replay** of the prompt.
4. `cat ~/.vitriol/officina/stalls/trail-<pid>.jsonl` — `stall.evaluate` carries
   `pi_flat`, `engine_busy`, `descendants`, which is the evidence that names the
   blocker.
5. With `OFFICINA_HOOK_TRACE=1`, the same drill writes
   `~/.vitriol/officina/hooks/*.jsonl`; the hook with `phase:"enter"` and no
   matching `exit` is the one that hangs the tool preflight — that is the last
   remaining unknown from this investigation (§2.3).

Rollback: `OFFICINA_SELF_HEAL=0` (old abort-and-release), and
`OFFICINA_STALL_GRACE_SECS=0` to make the ladder abort on first touch again.
