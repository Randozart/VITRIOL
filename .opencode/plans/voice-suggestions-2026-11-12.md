# Voice suggestions — ghost-text prompt prefill that learns your voice

- Date: 2026-11-12 17:10
- Status: implemented 2026-11-12 — stages 1–3 landed (352c835, 8402b9c, stage-3 commit). Telemetry verdicts simplified to accepted|dismissed-typed|dismissed-esc (ghost flow can't distinguish edits from accepts). Stage-3 note: safePrompts are session-scoped by design (I4 keeps it simple); cross-session voice memory rides the profile itself.
- Builds on: composer-predictions (heuristic widget line, 2026-11-12),
  hang-watchdog judge pattern (free-lane micro-completions), auto-route's
  privacy classifier.

## Behavior

After a turn ends, the operator's composer shows a **dimmed ghost text**
suggestion of their likely next prompt, phrased the way they actually ask.
Any typing dismisses it; **→** (right at end-of-input) accepts it into the
real draft; nothing is ever auto-sent. Over time the suggestions converge
on the operator's voice via two learning signals: which suggestions they
accept/edit, and a periodic distillation of their own **safe-classified**
recent prompts.

## Stages

1. **Suggestion engine** (`composer-predictions` v2, TS) — after
   `agent_end`, gather signals (verify-last, git porcelain, last assistant
   reply tail, voice profile if present) and generate the likely next
   prompt via the best free catalog lane (~120 tok, temp 0.3, 20 s
   timeout, fail-open). Heuristic priority: the existing rules
   (gates→fix, dirty→commit) override the LLM draft when they fire. The
   suggestion lands in `~/.vitriol/officina/prompt-suggestion.json`
   (atomic: version, id, text, source llm|heuristic, created, expires
   10 min). Kill switch `OFFICINA_NO_PROMPT_SUGGEST=1` (the existing
   `OFFICINA_NO_PREDICTIONS=1` keeps governing only the widget line).
2. **Composer ghost text** (Rust TUI) — on the turn-end event the TUI
   reads the suggestion file; when the composer is EMPTY and idle, the
   text renders dimmed after the cursor. **→** at end-of-input accepts it
   into the real draft; any printable character or Esc dismisses. Enter
   is unchanged (ghost alone is not sendable). Acceptance telemetry —
   accepted / dismissed, with the suggestion id and source, never prompt
   text — appends to
   `~/.vitriol/officina/prompt-suggest-events.jsonl` (0600).
3. **Voice learning** (TS) — the extension keeps this session's recent
   user prompts in memory, dropping any the auto-route classifier does not
   mark privacy-safe. Every ~40 new prompts or once a day (day-capped,
   `OFFICINA_NO_VOICE_LEARN=1` kills learning only), a free-lane
   distillation folds accepted suggestions + safe prompts + the current
   profile into `~/.vitriol/officina/voice-profile.json` (versioned,
   ≤ 200 words: tone, register, verbs, length, language, quirks). Stage 1
   injects it into every generation prompt.

## Files

| File | Stage |
|---|---|
| `officina/.pi/extensions/composer-predictions/suggest.ts` + tests | 1 |
| `officina/.pi/extensions/composer-predictions/index.ts` | 1, 3 |
| `officina/tui/src/tui/{state.rs,layout.rs,mod.rs}` | 2 |
| `officina/docs/officina-guide.md` | rolling |

## Invariants

- **I1 advisory only** — ghost text is render-only until accepted; nothing
  is ever pre-sent; Enter never fires on ghost alone.
- **I2 composer ownership** — the ghost appears only when the composer is
  empty and idle; the first keystroke belongs to the operator (it
  dismisses the ghost and inserts their character).
- **I3 heuristic priority** — verify-failing / dirty-tree suggestions
  always outrank LLM drafts.
- **I4 privacy gate** — only auto-route-safe prompts ever reach a cloud
  lane for distillation; acceptance telemetry carries ids and verdicts,
  never prompt text.
- **I5 corrupt-safe facts** — suggestion/profile/events files are atomic,
  versioned, and unreadable-garbage means the feature is silently off.
- **I6 zero-env parity** — no new env set ⇒ today's behavior (widget
  heuristic line) unchanged; both kill switches work independently.

## Gates

- Stage 1/3: `npm test` + `npm run typecheck` + free-lane smoke.
- Stage 2: `cargo test` + `cargo fmt --check` + `cargo build --release`
  + install to `~/.local/bin/officina`.
