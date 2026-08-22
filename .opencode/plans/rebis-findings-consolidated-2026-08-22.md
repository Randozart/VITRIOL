# REBIS consolidated findings — everything measured, everything learned

**Date:** 2026-08-22 09:00
**Span:** TUI observability work → Phase 0 hardware truth → battery → gateway
**Companion docs:** EXPERIMENT_LOG.md (full entries), plans rebis-*,
rebis-phase4-report.md

## 1. Hardware truth (the foundation)

| fact | number | consequence |
|---|---|---|
| Mellum2 pins fully at 64k ctx on 1070 Ti | 6.87/8 GiB, **70.2 t/s** decode | SWA(3:1)+4 KV heads make long-context KV ~free; earlier "can't fit" estimates wrong |
| Qwen3.8 IQ2_S resident on 3060 beats IQ3_S streaming | prefill **1.7×** faster (428–438 vs 239–264 t/s), decode equal | Pascal DMA crossing removed from Qwen's path; frees 1070 Ti entirely |
| Co-residency interference | zero measurable | concurrent probes clean |
| Same-prefix cache reuse post-H1 | **46.95s → 0.06s** (99.9%) | prefix caching works when gated |
| System RAM is 15 GB | two OOM incidents traced here | mmap weights + bounded caches + staggered starts mandatory |

## 2. Architecture verdicts

- **Drafter selection matrix**: Mellum = new/small-file generation;
  Qwen = modifications to real files; deterministic tooling = mechanical ops.
  Basis: six-config delta-protocol bake-off — Mellum cannot emit
  verbatim-fidelity deltas or whole files >~250 lines at viable budgets.
- **Daily-driver shape**: hermes(Qwen brain) delegating through `rebis.py`
  as tool, or transparently through the Gateway v2. Same-task comparison:
  loop 130s vs direct 18m23s.
- **Mellum-direct under agentic harness**: not viable — hallucinates tools,
  under-calls, narrates instead of acting. Steering detects this reliably but
  cannot beat client timeouts. Capability exists (one run landed edits);
  reliability does not.
- **Qwen3.8 surgery proposals rejected**: KV-prune and SWA-retrofit solve
  non-problems on this config with fantasy-tier recovery budgets; MTP grafting
  redundant (native head already used).

## 3. Server-level fixes landed in the fork

- `--prompt-cache-min-lcp` (commit 025291f6e): cross-session bleed dead
  (root cause was sim = lcp/tokens_new inflating short prompts against large
  cached states); states ≤64 tokens always eligible.
- Validated triad: big session caches correctly · tiny probe answers clean
  beside it · same-prefix follow-up reuses (26.9s→1.2s).

## 4. Operational lessons (each cost a real failure)

1. Unbounded fork prompt cache (--cache-ram default 8192 MiB) OOM-killed
   servers → always set bounded values.
2. --no-mmap weight staging collided with a second load on 15 GB RAM →
   mmap weights, stagger starts.
3. `pkill -f` patterns match your own shell's argv → `[x]` bracket trick.
4. Tool-command timeouts kill process groups including setsid children that
   hadn't detached → separate launch/poll commands.
5. Mid-experiment git checkout of baselines poisons runs → snapshot copies.
6. /tmp/opencode is not durable storage for anything you need tomorrow.
7. Shared llama-server endpoints: interleaved clients evict each other's
   prefix states; tenants running killall nuke each other's servers.
   Role-dedicated endpoints required for concurrent use.
8. Another tenant's lifecycle management kills idle servers — day-long
   sessions need backend respawn resilience.

## 5. Loop defects found by the battery (all fixed in rebis.py)

Fragment-overwrite corruption (guard + backups) · verdict JSON broken by raw
newlines in evidence (sanitizer) · rustc first-error drowned in tail (ERROR
DIGEST) · correction turns regenerated from original slice instead of last
draft (current_files feedback) · verbatim-only invariant matching missed
paraphrases (id-based checks + hybrid fuzzy) · joint-satisfiability violations
in authored specs refused forever *by design* (twice) — authoring guidance now
mandates a satisfiability self-check · auditor hallucinated fixes for
already-present code at 17k prompt tokens (compiler_only mode added).

## 6. Data flywheel status

DistillRecorder live across file/patch/replace protocols and all exit paths;
shim logs judged/nudged/override/gateway turns. Verified capture on both
failure and success trajectories. Storage local-only (~/.vitriol/distill).
Training conversion deferred; format derives both SFT gold and DPO pairs.

## 7. Fine-tune scoping (see finetune-feasibility plan)

Local Mellum LoRA blocked (no unsloth arch support; MoE QLoRA unsupported;
bf16 ≈ 63GB). Sanctioned base: JetBrains Thinking-SFT checkpoint. Trigger:
when distill volume justifies one rented GPU hour.

## 8. Current topology

```
hermes ──► rebis :8280 (Hg=80)
             ├─ reason turns ──► Sol :8279 (Au=79, Qwen3.8)
             ├─ executor turns ► Luna :8247 (Ag=47, Mellum2)
             └─ finals/flagged ► pipeline (Luna drafts ∥ Sol ingests,
                                 constrained audit, native corrections)
TUI watches both heads; distill store compounds every turn.
```

## 9. Open items

- Day-long readiness build (see daylong-memory plan): rolling flags, gateway
  compaction, checkpoint bounds, backend respawn
- Anticipatio incremental stream-warming (one-shot warm shipped)
- Route-threshold tuning from accumulated distill data
- Endpoint ownership coordination with the box's other tenant
- D2 forge trigger review once harvest volume is material
