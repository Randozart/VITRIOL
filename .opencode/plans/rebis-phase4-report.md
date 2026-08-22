# Phase 4 report — REBIS acceptance battery

**Date:** 2026-08-22 08:15
**Status:** S1 cell complete; S2/H1' optional extensions pending
**Span:** plans `rebis-phase0-plan`, `rebis-phase2-4-agentic`,
`rebis-hermes-daily-driver`, `rebis-distill-harvest` (all 2026-08-21/22)

## What was asked

Prove the REBIS pattern — Mellum2 speed + Qwen3.8 oversight on asymmetric
dual-GPU — as a viable daily driver under real agentic use, with a formal
battery on real repo tasks.

## Hardware truth established first (Phase 0)

- Mellum2 i1-IQ4_XS pins fully on the 1070 Ti **at 64k ctx**: 6.87/8 GiB,
  70.2 t/s decode, 557 t/s prefill. SWA(3:1) + 4 KV heads make long-context KV
  nearly free; earlier "can't pin at 64k" estimates were wrong.
- Qwen3.8 UD-IQ2_S resident on the 3060 beats the IQ3_S dual-GPU streaming
  profile on BOTH axes: prefill 394–438 vs 239–264 t/s (no PCIe DMA crossing),
  decode equal within noise. The Pascal penalty for Qwen disappeared.
- Co-residency: zero measurable interference; concurrent-load probes clean.

## Battery results — task S1 (`SlotSnapshot::total_tokens`, cargo test gate)

| arm | path | result |
|---|---|---|
| A | hermes → Qwen direct | correct incl. tests, **18m23s** wall |
| B | hermes(Qwen brain) → rebis loop → Qwen drafter | **ACCEPTED iter 1, 130s**, 152/152 tests |
| B′ | rebis loop, Mellum drafter | fails on modify-tasks in all 6 delta configs |
| C | hermes → shim → Mellum w/ steering | FAIL ×2, precisely diagnosed |

Arm C diagnosis: Mellum under-calls tools / hallucinates tool names under an
agentic harness regardless of steering; steering layer fires correctly but
judge+nudge latency exceeded client timeouts (BrokenPipe crash found and fixed).

## Delta-protocol bake-off (drives guide §0)

Six configurations on the same task; full matrix in EXPERIMENT_LOG.md
2026-08-22 02:30 entry. Verdict: whole-file drafting caps at ~250 lines;
unified-diff drafting fails on hallucinated context at every temperature/
budget setting; SEARCH/REPLACE with few-shot example is the only protocol the
drafter class formats reliably — and even then verbatim fidelity exceeded its
grade, so modifications route to Qwen while Mellum keeps new/small-file
generation at 70 t/s.

## Incidents and hardening (all now guarded)

1. Fragment overwrite corrupted two real source files mid-battery → FRAGMENT
   GUARD (<25% of existing size rejected) + `.rebis-bak` snapshots.
2. Process-group self-kills via `pkill -f` pattern matching our own argv →
   `[x]` bracket trick; detached setsid launches.
3. Host-RAM OOM ×2: unbounded fork prompt cache (--cache-ram default 8192 MiB)
   and --no-mmap weight staging collision on a 15 GB RAM box → bounded caches,
   mmap weights, staggered starts.
4. Cross-session context bleed: fork prompt-cache restored unrelated states
   (short-prompt similarity bias) → H1 fix below.
5. Mid-experiment git checkout reverted a baseline under active runs → snapshot
   discipline codified in REBIS-GUIDE §6b.

## Loop defects found & fixed during battery

- Verdict JSON with raw newlines inside evidence strings → sanitizer retry
- Compiler reports kept rustc tail only (first error drowned) → ERROR DIGEST
- Correction turns regenerated from original slice instead of last draft
- Verdict coverage required verbatim invariant text → id-based checks +
  hybrid fuzzy fallback (sequence OR token-containment ≥0.7)
- Joint-satisfiability violations in authored specs refused forever by design —
  twice; authoring guidance now includes a satisfiability self-check

## H1 hard task — ACCEPTED

llama.cpp server prompt-cache min-LCP gate (`--prompt-cache-min-lcp`, commit
025291f6e). Root cause of cross-session bleed was metric bias: sim =
lcp/tokens_new inflates short prompts against large cached states. Functional:
8.8k session cached; tiny probe answers clean with state resident; same-prefix
follow-up 26.9s → 1.2s. **Reopens Anticipatio**: prefix reuse works post-gate.

## Training-data flywheel (D1)

DistillRecorder captures every run — full drafter texts, before/after file
snapshots, gate digests, verdict deltas, token spend — to
`~/.vitriol/distill/`; shim logs judged/nudged/override turns. Verified on both
failure and success trajectories. Local-only policy (embeds repo code).
Fine-tune feasibility scoped separately: local LoRA blocked on arch support +
VRAM; sanctioned base = JetBrains Thinking-SFT checkpoint when cloud hour is
warranted (see `rebis-finetune-feasibility` plan).

## Architecture verdict

Daily driver = hermes (Qwen brain) delegating mechanical implementation
through `rebis.py` as a tool — proven at 130s vs 18m23s direct. Drafter
selection per guide §0. Shim steering exists as instrumentation; not needed
for v1. The loop turns latency asymmetry into quality: cheap fast tokens for
volume, expensive slow tokens for judgment, compiler as referee.

## Remaining / optional

- Formal timing arms repeated across more tasks (pattern proven on S1)
- Anticipatio shadow-prefill revisit on gated cache machinery
- Unsloth `mellum` support watch for local D2 forge
