# Officina Algorithmic Support — Gap Analysis & Proposal

**Date:** 2026-08-31
**Scope:** deterministic ("algorithmic") supports we can wire into the Officina
extension layer so the agent spends less intelligence and less context.

## 1. What Officina already has (audited 2026-08-31)

The extension layer in `officina/.pi/extensions/` already implements most of
the classic deterministic-support stack. Inventory:

| extension | algorithmic job | memory effect |
|---|---|---|
| `diagnostics-loop` | fast syntax check after every edit/write; failures re-injected as a tail message (~300 tok) before the next LLM call | prevents error-oscillation turns |
| `rtk-output` | test/build output reduced to exit status + error lines + verbatim tail at ENTRY; raw payload parked in `.pi/rtk/<id>.log` | 60–90% reduction on command output |
| `tool-result-clearer` | `context` event evicts stale tool results (old reads, old test runs) before every LLM call | largest single context saver |
| `repo-map` | tree-sitter symbol graph + PageRank (Aider technique); ~500 tok structural overview replaces 5–10K of blind reads | replaces exploratory reads |
| `read-guard` (pi core) | caps large file reads | caps read bloat |
| `task-state` | task list lives in `.pi/tasks/<session>.json`, re-injected from DISK each turn | survives compaction by construction |
| `knowledge-inject` | scores `skills/knowledge/*.md` against the prompt, injects top within budget | cheap procedure knowledge |
| `small-lane` | compaction summarized by mellum2 on :8287, not the 27B master | minutes → seconds on compaction |
| `caveman` | deterministic compressor on sub-coder reports / memory retrieval (dark, `TRIS_CAVEMAN=1`) | compression of secondary text |
| `snapshot` | per-turn git snapshots under `refs/trismegistus/turns/` | free undo, no context cost |
| `memory` + `memory-extractor` | owned memory store + regex-rule fact candidates → curator queue | cross-session, not context |
| `injection-guard` | filters ingested web content before it enters context | prompt-injection + bloat |
| `subagent` / `deep-research` | isolated child coders with truncated reports | keeps scratch work out of main ctx |

Conclusion: the foundations are strong. The gaps below are what remains.

## 2. Proposed additions

### P1 — `format-gate`: deterministic formatter on edit (highest value/effort ratio)
On `tool_result` for edit/write, run the file's canonical formatter
(prettier / ruff format / gofmt / rustfmt / clang-format; detection by
project-marker files: `package.json`, `pyproject.toml`, `go.mod`,
`Cargo.toml`, `compile_commands.json`). Behavior:
- Format in place; if the formatted content differs from what the model
  wrote, append a one-line notice ("file reformatted by <tool>; on-disk
  content is canonical — do not re-edit for style").
- Never echo a full diff (context cost); the next `diagnostics-loop` pass
  already validates.
Why it helps: style reasoning is pure token waste for the model; a small
local model in particular will otherwise burn turns matching brace style.
Kill switch: `OFFICINA_NO_FORMAT=1`, same pattern as `TRIS_NO_DIAGNOSTICS`.

### P2 — `edit-churn-watchdog`: repetition detector
Track (file, old→new content hash) pairs across the session. On the 3rd
semantically-identical edit round-trip (same hash pair recurring), emit a
tail directive: "you have applied this exact edit N times; the edit is not
sticking — read the file fresh or change strategy." Also cap total edit
count per file per session with a soft warning at ~10.
Why: small models loop; loops are the most expensive failure mode (each
iteration is a full LLM call with a full context). Purely algorithmic —
hashing only, no LLM.
Kill switch: `OFFICINA_NO_CHURN=1`.

### P3 — `import-lint`: cheap undefined-import check before diagnostics
For Python and JS/TS, a regex/AST-light pass that collects referenced
identifiers vs. imports in the edited file and flags obvious
missing/unused imports. Sits one rung below `diagnostics-loop` (which
delegates to `tsc`/`pyflakes` when present); this one works when no
toolchain is installed, which is exactly the small-model workshop case.
Budget: same ~300 tok tail as diagnostics.

### P4 — `verify-contract`: deterministic post-edit invariants
Extends the diagnostics loop with project-level cheap checks on edit:
- JSON/YAML/TOML parse validation for config edits (trivial for an
  algorithm, a real failure mode for models).
- For `package.json` edits: JSON schema sanity (name/version/dup keys).
- For Python: `python -m py_compile` when the formatter/linter is absent.
Output folded into the existing diagnostics tail, not a new channel.

### P5 — `diff-fidelity-guard`: verify patch application
Before accepting an edit tool result, re-read the file and confirm the
intended old-string actually changed; if the tool reports success but the
content hash is unchanged (silent no-op, common with fuzzy matchers),
block with `block: true` on the next attempt and surface the mismatch.
Pairs naturally with the churn watchdog (P2): no-op edits are its cause.

### P6 — `session-ledger`: deterministic end-of-turn stats injection
One compact line before each turn boundary: tokens-in-context (pi exposes
this), files touched, tests run/failing count. Gives a small model a
stable anchor for "where am I" without it re-reading state.
Optional; `OFFICINA_NO_LEDGER=1`.

## 3. Not recommended

- **Full lint suites (eslint/ruff full run) per edit**: too slow and too
  chatty for the per-edit loop; `diagnostics-loop`'s fast single-file
  checks are the right granularity. Full lint belongs to the user's
  pre-commit flow or a `/verify` command.
- **LLM-based review passes**: contradicts the goal; the subagent/dispatch
  path already exists when a second opinion is genuinely needed.
- **More aggressive auto-clearing than `tool-result-clearer`**: risk of
  evicting results the model still needs; the current entry-filter
  (rtk) + eviction split is the correct layering.

## 4. Implementation notes

All proposals follow the established house pattern (visible in every
existing extension): `tool_result`/`context` event handlers, tail-message
injection, env-var kill switch, char budgets, `harnessEvent` emission via
`_shared/events.ts`. Order of work: P1 → P2 → P5 → P4 → P3 → P6.
Each is independently shippable.
