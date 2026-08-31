# Rules & cache policy — instructions as a budget line

**2026-08-30.** Design outcome of the daily-driver diet discussion. Decisions
recorded here are binding for implementation; open preferences are marked.

## 1. The cost model (what caching does and does not remove)

Measured basis (this session): Hermes fixed overhead ≈ **19,953 tok** on a
trivial turn; `prompt-size --json` breakdown of the 119K-char system prompt:
context files **97,273 B**, stable identity 8,850 B, volatile 12,917 B,
tool schemas 52,604 B, 79 skills indexed. Engine prefix cache proven live:
prompt-tokens 7,870 → 1,722 across identical runs.

Caching eliminates **repeated prefill compute** — nothing else. Three costs
survive a cache hit:

1. **Window residency** — KV is allocated per occupied token forever
   (Rule 5: WINDOW ≠ DEPTH). VITRIOL makes residency cheap (q4_0 KV,
   sparse-LULL) but the budget table counts it regardless.
2. **Attention dilution** — per forward pass, cache hit or not. A weak model
   skims 30K of docs every turn, for free, and still gets it wrong.
3. **Bust exposure** — cache value ∝ 1/edit-frequency. Repo docs are the
   most frequently edited content in any project; the 97K block is
   therefore *the worst possible cache resident*: large, mutating,
   situationally relevant only.

Conclusion: **the cache is a budget line; only slowly-changing,
always-relevant content earns residency.** Hermes' structural split
(stable / context / volatile) is correct — the bug is what we let into the
stable block.

## 2. The tier model as cache policy

| Block | Content | Edit cadence | Cost |
|-------|---------|--------------|------|
| **Contract** (resident, cached) | distilled rules steering every turn | rare → cache nearly always warm | permanent window, ~1.5–2K tok — worth it |
| **Index** (resident, cached) | generated one-liners + triggers + paths (from `tris rules-index`) | changes only when headings change → cache-safe | ~0.6K |
| **Bodies** (never resident) | full AGENTS.md sections, repo docs | constantly | 0 window; retrieved slices ≤1K, transient, evicted by clearer like any tool result |

AGENTS.md remains the single authoring surface (writer habit unchanged);
`tris rules-index` is the *compiler* that splits it into cache-resident
artifacts and retrievable bodies. Content is never lost — only repackaged.

**Enforcement beats instruction (Tier 2):** every checkable rule becomes a
gate (validate, praetor, permissions-guard, smoke) and its prose duplicate
is deleted from resident tiers. Proven exchange rate this session:
permissions-guard, perms-mirror staleness FAIL, supervisor-aware stop all
replaced prose rules that a 27B would violate anyway. Gates cost 0 tokens.

**Tier 1 containers already exist:** Hermes' skill system (index line ≈70 B,
body on invocation) is the retrieval engine for operational rule-sections;
pi side uses knowledge-inject (200-tok cap) + read.

## 3. The knobs (decided: configurable, not fixed)

Residency size is **data-dependent** — build the knob, ship a reversible
default, let the dogfood day decide from the ledger.

```yaml
coding:
  rules_pipeline:
    contract:  { enabled: true, budget: 2000, source: AGENTS.md }
    index:     { enabled: true, budget: 600 }
    bodies:    { enabled: true, slice_budget: 1000 }
    cache:     { bust_events: true, frozen_prompt: false }
```

- **Budgets are hard**: `tris rules-index` refuses to emit an over-budget
  contract (the generator is the gate; overflow forces distillation or
  demotion to bodies).
- **New stage → new kill switch** (Rule 15): all four `enabled` flags join
  the validator inventory; missing switches FAIL `trismegistus validate`.
- **Defaults are provisional and marked**: `contract: 2000` chosen for
  asymmetric risk (oversize = certain silent cost; undersize = observable,
  one-number fix). Config comment carries date + "decide from ledger
  post-R3".
- **Measurement loop**: `prompt-size --json` (configured budget vs measured
  resident size) + engine prompt-tok deltas (cache hits, bust spikes) →
  ledger rows → cockpit cache panel. After a real day: raise, lower, or
  keep — with numbers.
- **Toolset/skill pruning** uses the same pattern: `toolsets: [...]` config
  list, every cut measured with `prompt-size` before/after, reversible
  per-line. Target: fixed overhead ≤ 8K tok (from ~20K).

## 4. Open preferences (not silently encoded)

1. **Packaging**: markers-in-one-file (AGENTS.md grows section markers +
   generated index header; authoring habit and human diffs preserved) vs
   sections-as-source (AGENTS.md generated from docs/rules/*.md).
   **Lean: markers-in-one-file** unless overridden.
2. **Residency size** — decided from ledger after R3, not now.
3. **frozen_prompt** — engine knob surfaced in config; verify the fork's
   wiring (see §5) before enabling.

## 5. Verify-first items (read source before leaning on any of this)

1. Hermes `context_file_max_chars` truncation semantics — a half-loaded
   rule is a corrupt rule; must know what a cap actually does.
2. Hermes skill auto-load matching (how index lines trigger) — determines
   how rule-index lines must be written.
3. VITRIOL `frozen_prompt` fork wiring (profile key exists; confirm it
   pins the system prefix KV as intended).

## 6. Execution order (updated)

1. Verify-first items (§5) — read-only
2. Diet machinery: `rules_pipeline` config block + validator inventory +
   `tris rules-index` (budget-gated compiler) + compile our AGENTS.md +
   toolset list knob + before/after into ledger
3. Track A staging (sudo; user's shell or explicit approval): stage 580xx
   against 6.18.42 only, `dkms status` verified pre-boot; reboot becomes a
   formality (removes 7.1.8 kernel first so DKMS never builds against it)
4. Acceptance: interactive `tris code`/`tris chat` PTY smoke; cache/bust
   panel in cockpit; `hermes backup` timer; mongod-vitriol loop decision
5. Reboot → 27B CERT → R3 dogfood day: settles residency, parallelism
   ceiling (R3.5b), and the empirical "better?" question
