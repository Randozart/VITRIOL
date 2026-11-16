# "Recommended next" quality + composer key handling (3 operator reports)

- Date: 2026-11-16 (investigation 07:45–08:10 local)
- Mode: PLAN (read-only). No code changed; this is the plan.
- Baseline commit: `84bcf54` (stall hardening), installed TUI `8b07965b…`.

Three separate defects, all reproduced by reading the code paths that the
operator exercised. Each has one root cause; none is a UI-polish issue.

---

## 1. "The recommended next text spills over the LLM instructions instead of
## recommending next steps in line with the user's request"

### Evidence

Live artefact (`~/.vitriol/officina/prompt-suggestion.json`):

```json
{"text":"▸ next: commit the work (95 changed files)","source":"heuristic",…}
```

That is the *entire* recommendation, and it is a **process rule from AGENTS.md**
("Git commit after every stage"), not a step of the work the operator asked for.
`prompt-suggest-events.jsonl` shows the rest of the day's suggestions being
dismissed as typed/history, i.e. the operator rejects nearly all of them.

### Root cause (two halves)

`officina/.pi/extensions/composer-predictions/predict.ts:15-27`:

1. `predict()` ranks **harness state** first and unconditionally: failing gates →
   `▸ next: fix the failing gates — … (/verify)`; dirty tree →
   `▸ next: commit the work (N changed files)`; only `null` otherwise.
2. `suggest.ts:30-35` `resolveSuggestion()` gives the heuristic **absolute
   priority (I3)** — so whenever the tree is dirty (i.e. most of the time, since
   the agent edits files for a living) the LLM draft is *discarded* and the
   operator only ever sees process hygiene.

And the draft path has no idea what the operator asked for:

- `suggest.ts:64-70` `GenSignals` **declares `taskTail`** but
  `index.ts:311-320` never populates it — the only signals passed are
  `lastAssistantTail`, `verifyKnown/verifyOk`, `dirtyFiles`.
- No signal carries the operator's request (last user prompt) or the task list's
  in-progress item, so even a good free-lane reply is pure guesswork from the
  assistant's tail.
- `voiceBlock()` (suggest.ts:52-60) is the only "style" input, and it is absent
  today (`voice-profile.json` does not exist), so the draft lane runs on a
  generic prompt with a weak free model — the "instruction spill" reading of a
  draft that mimics agent-facing text.

So: the surface is *state-driven*, never *request-driven*, and the ranking
guarantees the state-driven answer wins.

### Decisions (locked)

- **D1** Task progress outranks process hygiene. Order: in-progress task →
  follow-through on the operator's last request → process hint (as a *suffix*,
  never the whole line).
- **D2** The process hint survives but demoted: `▸ next: <task step>` plus
  `· 95 files to commit` appended when dirty. It only stands alone when there is
  no task and no request to follow.
- **D3** Reject instruction-shaped output. A suggestion must not be a harness
  directive: drop drafts/heuristics containing `AGENTS.md`, `praetor`, `gate(s)`,
  `/verify`, `GRIND:`, `commit the work`, `Stage N`, "instructions", or a bare
  imperative about repository bookkeeping.
- **D4** Silence beats noise: if the operator's last prompt is a question
  ("what's next?", "resume") and there is no task list, publish **nothing**
  rather than a process rule. The ghost disappears; no stale line.
- **D5** Never send; advisory only (I7 of the existing contract is preserved).

### Plan

1. `suggest.ts`
   - Extend `GenSignals` with `userPrompt?: string` (last non-slash prompt,
     ≤ 240 chars) and populate `taskTail` (already declared).
   - `buildGenPrompt`: new signal block, ordered
     `in-progress task → operator's last request → assistant tail → process
     state`, with the process lines last and clearly labelled "hygiene".
     Tell the model explicitly: "Do not restate harness rules or process
     instructions; propose the next step of *this* work."
   - New pure `suggestionAllowed(text): boolean` implementing D3, applied to
     **both** the heuristic and the draft (D3 covers the heuristic too).
   - `resolveSuggestion(heuristic, draft, signals)`:
     heuristic wins only when it is task-shaped; otherwise the draft; otherwise
     `null` (D4).
2. `predict.ts`
   - `predict(verify, dirty, now, task?: string)` returns
     `▸ next: <task>` when a task is in progress; appends
     `· N changed file(s)` / `· gates failing (/verify)` as a suffix (D2);
     returns `null` (not the commit line) when there is no task and the tree is
     dirty only.
   - Keep the gate-failure variant when gates are actually failing **and** no
     task is in progress (it is the one process hint with immediate value).
3. `index.ts`
   - Track `lastUserPrompt` on `message_end` (role `user`, non-slash, ≤240).
   - Import `getTaskItems` from `../task-state/index.ts` (its state object is
     `globalThis`-backed — `__officinaTaskFileState` — so a second jiti instance
     reads the same file; this is the documented cross-extension pattern) and
     take the `in_progress` item, falling back to the first `pending` one.
   - Pass both into `predict()`/`buildGenPrompt()`; the widget row and the ghost
     share one line, so the sidebar row stops contradicting the ghost.
4. Tests (`predict.test.ts`, `suggest.test.ts`, new `suggest-gate.test.ts`)
   - Golden matrix: (task, userPrompt, verify, dirty) → exact line. Pins D1/D2.
   - D3 matrix: instruction-shaped text rejected for heuristic **and** draft.
   - D4: question prompt + no task ⇒ `null`, and `prompt-suggestion.json` is
     cleared (not left stale).
   - Regression: an empty task list + clean tree still yields `null` (no
     invented advice).

### Invariants

- **N1 parity**: with `OFFICINA_NO_PREDICTIONS=1` nothing changes.
- **N2 advisory only**: suggestions never auto-send, never touch the composer
  until the operator accepts.
- **N3 no instruction echo**: no staged suggestion may match the D3 vocabulary
  (pinned by the gate matrix).
- **N4 silence is valid**: `null` must clear the ghost, not leave the previous
  line visible.

---

## 2. "Up-arrow hangs on `/`, blocking the rest of the history. Should `/` spawn
## a small selection menu? Same with `/q`, so I don't have to enter it twice"

### Evidence / root cause

`officina/tui/src/tui/mod.rs:1807-1821`:

```rust
KeyCode::Up => {
    if state.cand_sel_logic_open() { state.cycle_candidate(false); }
    else { state.history_prev(); }
}
```

`history_prev()` (`state.rs:2067-2085`) writes the recalled entry into
`state.input`. This operator's history is full of slash commands
(`/mo`, `/model`, `/q`, `/quit`), so the moment an entry starting with `/` is
recalled, `cand_sel_logic_open()` becomes true
(`state.rs:4483` = `!command_candidates().is_empty()`) and **every later
Up/Down cycles the popup instead of the history**. The popup also renders over
the recalled line (`layout.rs:932-945`). That is exactly "hangs on /".

`/q` twice: `mod.rs:2272-2286` — Enter on a *partial* command word completes it
and `return`s (no send), so `/q` → `/quit` and then a second Enter to execute.
`LOCAL_COMMANDS` (`state.rs:4275`) has a single `q`-prefixed command, so the
ambiguity check already knows `/q` is unique — it just chooses "complete"
instead of "execute".

### Decisions (locked)

- **D6** History recall never opens the popup and never steals arrows.
- **D7** `/` opens a real menu when the operator is *typing* a command: rows,
  highlighted selection, Enter executes, Esc closes the menu (and does not touch
  history).
- **D8** A unique partial command **executes on Enter in one press**
  (`/q` → quit, `/mo` → model picker) with a visible notice
  (`/q → /quit`). Ambiguous prefixes still complete; Tab still completes.

### Plan

1. `state.rs`
   - `pub history_recall: bool` on `AppState`; set in `history_prev`/
     `history_next`, cleared by `push_char`/backspace/paste/`accept_ghost`/
     `clear_input`/send (one helper `fn note_input_edited(&mut self)` called
     from the composer's text-mutation paths).
   - `cand_sel_logic_open()` → `self.command_menu_open()`:
     `!history_recall && !command_candidates().is_empty()`.
   - `unique_command(&self, word) -> Option<&SlashCommand>` for D8.
2. `mod.rs`
   - Up/Down: `history_recall` wins over the popup (D6).
   - Esc: while the menu is open, close it (`menu_closed_for` marker) instead of
     being a no-op (D7 second half).
   - Enter on `msg` that is a unique partial command → dispatch that command
     directly (D8), keeping the existing "unknown command" rejection for
     non-unique gibberish.
   - Notice text `format!("{msg} → /{}", cmd.name)` so the expansion is visible.
3. `layout.rs`
   - Popup is a menu only when `command_menu_open()`; render it with the
     selected row highlighted and a one-line footer
     (`↑↓ select · Enter run · Esc close`) — no behavioural change beyond D7.
4. Tests
   - `state::tests::history_recall_does_not_open_the_command_menu`:
     recall `/model`, assert `command_menu_open()` false and that
     `history_prev()` still walks.
   - `state::tests::editing_after_recall_reopens_the_menu`.
   - `tui::tests::enter_runs_a_unique_partial_command` (`/q` → `Quit`) and
     `enter_completes_an_ambiguous_partial`.
   - `layout::render_smoke`: menu renders at 80×24 and 120×40.

### Invariants

- **K1** Arrow keys always reach history when the line came from history.
- **K2** No command can execute by accident: only a *unique* prefix executes,
  and the notice names the expansion.
- **K3** Tab semantics unchanged (complete/cycle, never send).
- **K4** Esc order: modal → menu → (idle) no-op → ^esc quits.

---

## 3. "Esc doesn't properly end the work"

### Evidence / root cause (three distinct faults)

1. **Queued work is re-sent right after the abort.**
   `mod.rs:698-720`: on `agent_end`, if `!state.queued_prompts.is_empty()` the
   next message is sent immediately. Esc aborts the turn; the very next
   `agent_end` starts the queued turn — the work continues by design.
2. **Esc escalates to a pi respawn.** `mod.rs:1680-1694` sends `Abort` and arms
   `abort_deadline` (10 s). That deadline is the *watchdog's* escalation point
   (`mod.rs:2666-2700`), and with self-heal on it ends in
   **`restarting pi`** — a deliberate interrupt must never kill the child.
3. **Pi-side held messages are not cancelled.** `Alt+Enter` sends
   `RpcCommand::FollowUp` and a prompt typed during a tool sends `Steer`; both
   live inside pi, so after an abort pi can start them. The TUI clears its own
   `held` list only on `agent_end`, never on Esc.

### Decisions (locked)

- **D9** Esc means "stop everything now": abort + drop queued prompts + drop
  held/steer messages + cancel dialogs + clear stall/replay state.
- **D10** Only the **watchdog** may escalate to a respawn. An operator abort
  releases the UI and states what is still finishing; `/recover` stays the
  manual escalation.
- **D11** Esc never loses typed text: the composer and its history are
  untouched; the dropped queue is reported so nothing disappears silently.

### Plan

1. `state.rs`
   - `pub enum AbortSource { Operator, Watchdog }`; `abort_deadline:
     Option<(Instant, AbortSource)>`.
   - `fn operator_stop(&mut self, dropped: usize) -> usize`: clear
     `queued_prompts` + `held`, clear stall state, `note_prompt_failed()`,
     return how many were dropped.
2. `mod.rs`
   - `KeyCode::Esc` (streaming): `cancel_pending_dialogs`, `Abort`,
     `operator_stop`, arm the deadline with `AbortSource::Operator`, notice
     `"stopped — dropped N queued message(s)"` when N > 0.
   - The settle-deadline branch (`maybe_heal`): `Operator` → release the UI +
     `harness: pi is still finishing <tool> (<age>s) — /recover restarts it`;
     `Watchdog` → the existing respawn ladder, unchanged.
   - The `agent_end` queue-drain must not fire for a prompt queued before an
     operator stop (guard on a `stopped_at` seal cleared by the next operator
     send).
3. Pi-side follow-ups: after an operator stop, drop `pending_replay` and refuse
   to re-send `held` entries; add a `Diag` line when pi reports a post-stop
   `turn_end` so the operator can see the child was still busy.
4. Tests
   - `tui::tests::operator_esc_drops_queued_and_never_respawns`.
   - `tui::tests::watchdog_settle_still_escalates`.
   - `state::tests::operator_stop_keeps_the_composer_and_counts_the_drop`.
   - Manual drill (after landing): Alt+Enter twice during a long tool, press
     Esc → exactly zero further turns, no respawn, one explicit notice.

### Invariants

- **E1** An operator abort can never terminate or restart the pi child.
- **E2** An operator abort can never start a queued turn.
- **E3** Watchdog recovery is untouched (same thresholds, same budget).
- **E4** No silent loss: every dropped message is counted in one notice.
- **E5** ^esc (quit) and modal Esc semantics unchanged.

---

## Suggested order of work

1. **§3 Esc** — smallest, stops the session doing work the operator did not ask
   for (and it shares the abort path with the stall work landed in `84bcf54`).
2. **§2 composer keys** — pure TUI, no model dependency, immediately testable.
3. **§1 recommendation quality** — needs the task/request signals and a gate;
   largest behavioural change (it changes what the operator sees every turn).

Each stage: `cargo test` / `cargo fmt --check` (TUI) or `vitest` + `tsc`
(extension), then rebuild + reinstall the TUI, then the drill. Separate commits
per stage (AGENTS: commit after every stage).

## Open questions (not blocking, decided as noted)

- Should the dropped-queue notice survive as a chat entry (audit trail) or only
  as the transient notice row? **Plan: both** — a `Diag` entry plus the notice.
- Should `/` also accept a leading `!` for shell-ish inline bash (existing
  feature)? **Plan: unchanged**, out of scope.
- Do we want a `▸ next` line at all when the operator is mid-task and nothing
  new is provable? **Plan: yes, the in-progress task step**, because that is the
  request-aligned content the operator is asking for.

---

## As landed (2026-11-16, 08:15)

Three commits, one per issue, tests green, TUI rebuilt and installed
(`~/.local/bin/officina` md5 `4a54aeba…`; the previous live build was `8b07965b…`).

| commit | scope |
|---|---|
| `6ffc6d0` | Esc = operator stop (AbortSource, queue/held drop, no respawn) |
| `72d973d` | history recall keeps the arrows; unique command runs in one press |
| `922c6e0` | recommendation follows the work; instruction echoes rejected |

### §3 as landed
- `state::AbortSource { Operator, Watchdog }` tags `abort_deadline`; only the
  watchdog reaches `recover()`. An operator stop releases the UI and prints
  `harness: stopped — pi is still finishing <tool> (<n>s); no restart was
  attempted (use /recover if it stays stuck)`.
- `operator_stop()` drops queued prompts, held messages, expected tool calls,
  stall parking and any pending replay; the count is reported once.
- Esc acts when streaming **or** a tool is running **or** messages are queued.
- Tests: `operator_stop_drops_the_queue_and_recovery_obligations`,
  `abort_source_decides_who_may_escalate`.

### §2 as landed
- `history_recall` + `menu_dismissed_for` on `AppState`; `cand_sel_logic_open()`
  is now false for a recalled or dismissed line, so Up/Down always reach history.
- Esc closes the menu first (K4 order), leaving the composer untouched.
- `expand_unique_command()`: `/q` → `/quit` executes in one Enter with a
  `"/q → /quit"` notice; ambiguous prefixes still complete; Tab never sends while
  the composer holds text.
- Tests: `history_recall_does_not_open_or_freeze_the_command_menu`,
  `dismissed_command_menu_stays_closed_until_the_line_changes`,
  `a_unique_partial_command_expands_and_executes_in_one_press`.

### §1 as landed (and one decision amended)
- **D2 amended**: process hints never stand alone. The gate/commit line was
  itself the complaint ("commit the work (95 changed files)"), so `predict()`
  now returns `null` without a task and only *annotates* a task step
  (`▸ next: <task> · 1 gate failing · 3 changed files to commit`). The draft
  (which now sees the operator's last request) fills the silence, or nothing does.
- `GenSignals` gained `userPrompt` and now really populates `taskTail` from
  task-state's `getTaskItems()` (in-progress, else first pending).
- `suggestionAllowed()` rejects doctrine for both sources; `cleanDraft()` treats
  `SKIP` as silence; `null` clears the staged ghost.
- Tests: predict/suggest matrices + `runtime.test.ts` (end-to-end through
  `agent_end`: task wins, no-task clears, doctrine rejected, work-shaped draft kept).
- The runtime test writes into an isolated HOME imported *after* stubbing; the
  earlier version polluted the operator's real `prompt-suggestion.json` (it is
  cleared now, and the extension resolves its dir at import time).

### Gates
`cargo test` 284 · `cargo fmt --check` clean · `npx tsc --noEmit` clean ·
`vitest composer-predictions` 29 · `vitest _shared` 211 (with the operator's
`OFFICINA_ROUTER_CORE=rust` unset — that env var makes `core-rpc.test.ts` fail
before any change of ours) · Praetor: 0 new diagnostics on all three commits.

### Still open (not in this scope)
- pi-side queued follow-ups: after an operator stop the TUI drops its own copy,
  but a `FollowUp` already accepted by pi may still run. Observed behaviour is
  reported in the transcript; cancelling it needs a pi-side API (open question).
