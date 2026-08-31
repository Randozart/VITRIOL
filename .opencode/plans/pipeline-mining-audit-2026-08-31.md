# Pipeline Mining Audit — pi-coding-agent examples/extensions (79 files)

**Date:** 2026-08-31
**Question:** which of upstream's 79 example extensions add real workflow
value to Officina without becoming auto-injected noise?
**Method:** each file read and judged against four gates: (1) fits the small
local-model workflow, (2) not already covered by an Officina extension,
(3) deterministic or user-initiated — never pushes tokens on its own,
(4) maintenance cost below value.
**Verdict summary: 4 adopted (incl. ask, vendored earlier today), 2 deferred
with triggers, ~20 covered by existing work, rest rejected as demos/noise.**

## Adopted (this pass)

| example | vendored as | value |
|---|---|---|
| `question.ts` | `ask/` | agent can ask the user a clarifying question (options + free text) mid-turn instead of guessing. Essential for a 27B. |
| `notify.ts` | `notify-done/` | native terminal ping on `agent_end` (OSC 777/99). For multi-minute local decode, "done, waiting for you" is the QoL gap between local and hosted agents. Rebranded, require→ESM. |
| `inline-bash.ts` | `inline-bash/` | `!{cmd}` expansion inside prompts (whole-line `!cmd` passthrough preserved). Removes the copy-paste loop between terminal and composer. |
| `session-name.ts` | `session-name/` | `/session-name` — named picker rows. Direct response to the "can't find my session" incident. |

All four: citation headers, kill switches actually wired (Rule 15), NOTICE +
PROVENANCE updated.

## Already covered — do not vendor (duplication is noise)

| example | covered by |
|---|---|
| git-checkpoint, auto-commit-on-exit | `snapshot` (per-turn refs, non-destructive — auto-commit rejected as riskier) |
| todo | `task-state` (disk-backed, compaction-proof) |
| custom-compaction, trigger-compact, summarize | `small-lane` (mellum2 compaction policy) |
| permission-gate, protected-paths, confirm-destructive | `permissions-guard` + `agent-mode` write gate |
| handoff | `context-relay` (card handoff) |
| status-line, custom-footer, model-status, working-indicator, titlebar-spinner | `session-panel` + `vitriol-decode` (engine-truth gauges, not cosmetic spinners) |
| custom-header | `officina-header` (braille watermark) |
| qna, questionnaire | `ask` (single-question v1; questionnaire's multi-question flow is a v2 option) |
| send-user-message | `subagent` (and deliberately absent — see the mode-switch lesson) |

## Deferred (real value, unmet trigger)

| example | trigger to revisit |
|---|---|
| dynamic-tools, kimi-deferred-tools | if the registered tool count grows enough that tool schemas cost real per-turn tokens (today: ~15 tools, not yet worth it) |
| structured-output | when background-lane v2 (churn investigator) needs reliably-parseable verdicts |
| file-trigger | when read-ahead digests graduate from research to build |
| git-merge-and-resolve | when a merge-conflict session actually needs it |
| github-issue-autocomplete | when Ontic/VITRIOL work moves to `gh` issue flow |
| dirty-repo-guard | if snapshot-vs-worktree confusion ever bites in practice |
| reload-runtime | pi core already hot-reloads auto-discovered extensions; our launcher binds by flag — adopt only if we move extensions to auto-discovery |
| bookmark, modal-editor, prompt-customizer | owner-preference editor QoL — say the word |

## Rejected (demo/noise/risk)

hello, commands, tools, pirate, rainbow-editor, event-bus, entry-renderer,
message-renderer, built-in-tool-renderer, overlay-test, overlay-qa-tests,
rpc-demo, hello — demos of the extension API, zero workflow value.
mac-system-theme (wrong OS), ssh (remote-exec scope creep; also a security
surface), bash-spawn-hook (superseded by rtk-output's entry filtering),
system-prompt-header (we never touch the system prompt — KV discipline),
minimal-mode, hidden-thinking-label, widget-placement, working-message-test
(animations/persistence cosmetics or test rigs).

## The noise principle applied

Every adopted extension is **user-initiated or event-terminated** (it fires
when work *finishes*, or when the user types a prefix). None injects tokens
into context on a timer or on every turn. The injectors we already run
(skills, knowledge, diagnostics, ledger) each earn their ~300-token budget by
replacing a worse outcome (blind reads, failed edits, lost orientation);
that bar — *replace a bigger cost or enable a decision* — is the adoption
criterion going forward.
