# LoRA Workstream Final Report: Shared Reasoning Adapter on the Daily Driver

**Date**: 2026-10-12
**Status**: CLOSED — verdict **SHELVED**, owner accepted
**One-line**: the only credible shared Qwen3.8-base reasoning LoRA converts,
loads, and runs on the daily driver, but buys no capability, costs −16%
decode, and fails the owner's "faster + compiler-catchable = win" thesis in
a controlled compile-gated A/B.

## Origin and question

Owner goal: better **language-design and systems-programming** capability
from the daily driver (`qwen38-buffedmod-262k`, blessed 2026-09-28).
Preference: use an existing shared adapter if one exists — this is personal
use; no custom training unless necessary.

## Survey verdict (200 repos scanned)

- **Language-design adapters: zero exist.** Niche domain — capability here
  can only come from training or prompting, never borrowing.
- Genuine convertible 3.8-base adapters with any relevance: exactly three
  (`rico03` Opus-reasoning-distill, `nickzin` tool-calling control,
  `autotrust` JEV decision-model). Everything else relevant (Swift-1.5,
  iCoder, MusaCoder, jedisct1-rust, Fable distills...) is a **merged
  full finetune** = base-model swap + full re-cert, the opposite of cheap.
- Chosen candidate: `rico03/Qwen3.8-27B-Claude-Opus-Reasoning-Distilled`
  (PEFT r32/α32, Apache-2.0, sha256-verified, Unsloth-trained on 3.8 base,
  targets attn + GDN + MLP).

## What was built (durable)

| artifact | value after closure |
|---|---|
| Inner commit `cd55f1f3a` | converter fix: linear-attention v-head reorder on LoRA pairs (row→B, col→A permutation; verified diff 0.0). Upstream-grade bug — any future Qwen3.5-class adapter needs it |
| `scripts/eval-gate/{problems.py,run_gate.py}` | reusable compile-gate A/B harness: rustc + run + clippy tiers, K-round repair loop, per-attempt transcripts |
| 12-prompt prose eval set | `/tmp/opencode/lora/eval/prompts.py` (transient; embedded in round-1 report) |
| `~/Models/Qwen3.8-27B-opus-reasoning-lora-f16.gguf` | 467 MB, kept on the shelf; nothing depends on it |
| Fork gap documented | MTP draft `ctx_dft` never adapter-primed (`common/speculative.cpp:2609`) = −16% decode with any adapter; small isolated patch, NOT scheduled |

## Evidence summary

**Round 1 — prose A/B** (12 prompts, greedy, 400 tok; report:
`lora-tryA-opus-reasoning-2026-10-12.md`): consistent style shift
(denser, code-first, ~40% more ground per token budget); ~3/12 confident
slips in both prose and code (misdefined row polymorphism; broken OCaml
sig; `x/2→x>>1` claim; overflow-prone bounds check). Base: conservative,
occasionally truncates before the deliverable. Language-design
conceptual fidelity: adapter worse on definitions, better on synthesis
(ld2/ld3), uncatchable by any compiler either way.

**Round 2 — compile-gated agentic-loop emulation** (6 hard std-only Rust
problems; rustc + clean run + clippy −D warnings; K=3 stderr-feedback
repair; max_tokens 6000; reports: `lora-round2-compile-gate-2026-10-12.md`
+ `lora-round2-results-2026-10-12.md`):

| | base (unit) | adapter |
|---|---|---|
| greens | 1/6 (treiber) | 1/6 (spsc) |
| decode t/s | 36.6 | 30.9 (−16%) |
| tokens | 58.6k | 63.5k |
| wall | 1718 s | 2155 s |

Greens on *different* problems; MTP/batching nondeterminism (proven via a
discarded run) dominates at n=1 → no adapter edge measurable or claimed.
Neither arm converges borrow-checker failures at K=3 with whole-file
regeneration. Adapter tlv thrash: 22.7k tokens, no convergence.

## Final verdict and rationale

**Shelved, owner-accepted (2026-10-12).** The thesis "faster codegen +
compiler catches it = win" fails on its own terms: catching ≠ converging,
and the adapter's speed tax compounds exactly where repair loops need
throughput. The daily driver's economics won every comparison today
(36.6 t/s shallow decode, best cost-per-green). Live config untouched;
adapters list empty; unit active and healthy at closure.

## Lessons (condensed)

1. temp-0 is not deterministic under MTP/batching — repeat samples required
   for small-delta A/Bs.
2. Eval budgets must exceed problem size or they degenerate silently.
3. Whole-file repair loops stall on borrow-checker failures; real agentic
   loops need surgical edits and more rounds.
4. Check for foreign tenants (`pgrep -a llama-server`) before timing arms;
   pin speed claims to server-side decode timings.
5. Shared reasoning-distill adapters = style accelerant, not capability.
6. Niche-domain capability: no community shortcut exists.

## If ever revisited

- `ctx_dft` priming patch first (makes adapter evals tax-free).
- `scripts/eval-gate` with repeats (n≥3/problem) and surgical-edit repair
  (real agent) instead of whole-file regen.
- Custom training remains the documented fallback path (Kaggle 2×T4
  QLoRA, ~€0; data mix + eval plan drafted in the session history) —
  relevant only if a *capability* gap is ever proven, which today's
  shared-adapter evidence does not address.

## Provenance index

Fingerprints for every launch (unit untouched arms + manual `--lora`
arms with full argv) are embedded in the three predecessor reports and
EXPERIMENT_LOG. Commits: inner `cd55f1f3a` (converter fix); outer
`24af62d` (round 1), `03573e2` (round 2 + harness), plus this closure
commit.
