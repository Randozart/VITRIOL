# REBIS Mandatum authoring guide

*System documentation: `docs/REBIS.md`. This file covers only how to write task packets.*

How to write task packets the Rebis loop can actually converge on. Every rule
here was earned by a failed run.

## 0. Drafter selection matrix

| task shape | drafter | protocol |
|---|---|---|
| new file / small file (≤~150 lines) | Mellum2 (70 t/s) | `draft_mode: "file"` |
| modify existing real-world file | Qwen3.8 (~20 t/s) | `draft_mode: "replace"` |
| mechanical rename/move | deterministic tooling | not an LLM task |

Measured 2026-08-22: Mellum-Thinking cannot emit verbatim-fidelity SEARCH
blocks or correctly-counted unified diffs for files it must mirror (hallucinated
context, degenerate loops with thinking disabled, budget exhaustion with it on).
Qwen handles replace-mode reliably; when its draft misses a piece, the loop's
compile digest catches it in the next iteration.

## 1. Invariants must be JOINTLY SATISFIABLE

The verifier enforces each invariant literally and refuses the draft forever
if two invariants conflict. Before submitting a packet, check: can ANY file
satisfy all of them simultaneously?

Bad (contradictory — cost us a full failed run):
- "existing struct definitions unchanged"
- "the test must construct Ledger { items: vec![...] }"   ← needs pub fields or Default

Fix: grant explicit permission for what tests need:
- "you MAY add derives (e.g. Default) or constructors when tests need them"

## 2. Test-emitting invariants by default

Prose inspection is weaker than execution. Whenever a claim can be asserted
in code, make an invariant DEMAND the test:

- "the file contains a #[test] named total_sums_items constructing items
  [1,2,3,4] and asserting total() == 10"

Then set `compile_cmd` to run those tests (`cargo test`). The compiler gate
becomes the arbiter; the verifier's evidence check becomes the backstop.

## 3. Pin exact observable values

"sums all items" is auditable prose; "asserting total() == 10 for [1,2,3,4]"
is checkable fact. Prefer pinned numbers, exact names, concrete inputs.

## 4. Scope discipline

- One objective per packet. Multi-feature packets make correction orders
  ambiguous.
- List what must NOT change ("keep commit/reset semantics") — the verifier
  checks preservation as an invariant.
- Forbid scope creep explicitly when the drafter tends to add extras
  ("no new methods beyond total").

## 5. Slice hygiene

- `file_slices[].content` must be the CURRENT file state, not from memory.
- Keep slices small: only the functions/regions in play. Stable-prefix-first
  layout gives prefix-cache hits across loop turns.
- Multi-file tasks: one slice per file; the drafter emits each under a
  `### <path>` header.

## 6. Budget honestly

- `max_iterations: 3` is the default; hard tasks may need 4–5, but if you
  expect more than that the packet is probably underspecified.
- Set `compile_cmd` to the FASTEST command that gates the claim
  (`cargo check` for type-level, `cargo test` for semantic).
- `draft_budget`: file-mode drafts of >100-line files need ≥8192 for
  thinking drafters. Patch/replace modes: disable the drafter's thinking —
  deliberation burns budget the delta needs.

## 6b. Verification mode

`verify_mode: "compiler_only"` when every invariant is enforced by the gate
(test-emitting invariants, greppable structure). The LLM auditor on a big
prompt hallucinated fixes for code that already existed and invented
out-of-packet regressions — reserve it for genuinely semantic invariants.
Never `git checkout` baseline files mid-battery; snapshot with
`cp file /tmp/opencode/baseline/` instead (a checkout reverted a restored
file under an active experiment and poisoned three runs).

## 7. Hard-task protocol

For genuinely difficult changes: split into sequential packets along the
compile boundary. Each packet's gate-green output becomes the next packet's
slice content. A packet that cannot compile standalone is not a packet yet.
