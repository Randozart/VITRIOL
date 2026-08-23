# REBIS research notes — what the loop taught us

Condensed research findings from the battery and bake-off, for the TUI
GUIDE reader. Full detail: EXPERIMENT_LOG.md and .opencode/plans/rebis-*.

## Finding 1 — drafter selection is task-shaped

Six delta-protocol configurations produced one clean rule: Mellum2 drafts
new/small files superbly (70 t/s) but cannot mirror an existing real file
verbatim — every protocol (whole-file, unified diff, SEARCH/REPLACE)
degraded into truncation or hallucinated context. Qwen handles verbatim
modification reliably. Hence the route ladder.

## Finding 2 — verification must be split by provability

An LLM auditor at 17k prompt tokens hallucinated fixes for code that
already existed. A compiler cannot see semantic invariants ("the buffer
must be freed"). The answer is neither-or: `compiler_only` when tests/
greps enforce invariants; LLM audit only for genuinely semantic ones.
Both layers caught a poisoned sum independently when run together.

## Finding 3 — specs fail before drafters do

Two full loop failures traced to contradictory invariant sets ("definitions
unchanged" vs tests needing constructors). The verifier was *right* to
refuse forever. Author law: check joint satisfiability before submitting.

## Finding 4 — thinking tokens are budget thieves

Mellum-Thinking spends 40–70% of its token budget reasoning before
drafting. For precise-delta work, disable thinking; the spec carries the
reasoning already. For open-ended drafting, thinking pays for itself in
fewer pokes. Measure per task class.

## Finding 5 — shared inference endpoints thrash

Interleaved conversations evict each other's prefix caches; tenants that
`killall llama-server` between runs kill everyone's servers. Day-long
agents need role-dedicated endpoints or coordinated quiet windows.

## Finding 6 — the loop escalates honestly

Every failure mode observed ended in escalation, not garbage shipping:
unsound specs refused forever, iteration caps pause with journals,
unparseable verdicts become explicit correction orders. The system's
failure posture is its best feature.

PROVENANCE: all findings measured this repo, 2026-08-21/22 sessions;
EXPERIMENT_LOG.md entries 2026-08-21 20:40 through 2026-08-22 08:45.
