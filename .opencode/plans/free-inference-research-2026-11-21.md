# More free inference — research notes (2026-11-21)

Question: how can Officina benefit from MORE free (zero-cost) inference?
Current mix: 7 683 ledger rows — 79 % local, 21 % cloud, all cloud via the
free-candidate pool. 20 free-lane probe-ok lanes right now; the constraints
are elsewhere.

## What the system already does (evidence)

- **Free-only overflow by default**: `adaptive-router/index.ts:84-91`
  (`OFFICINA_OVERFLOW_CLOUD=free`, `free_only` pref) → pool discovered from
  the catalog via `toolCandidates` (`caduceus/catalog.ts:261-288`) gated by
  `FREE_OWNER_RANK` (`catalog.ts:188-204`, 15 ranked owners).
- **Background admission probes**: `cloud-probe/index.ts` — leader session
  pings lanes with a 1-token probe, publishes marks through the SAME
  parsers as real failures (`gateway-health.json` `probes`). Budgets:
  6/min, 2000/day (`cloud-probe/index.ts:54`).
- **Router park + lane walk**: `caduceus-router.mjs` with today's R1/R2
  fixes (phantom lane gone, parks capped at 900 s, unpark endpoint).
- **Catalog is deep**: 470 models, **201 ranked tool-capable free
  candidates** (measured from `caduceus-models.json`).

## Where free inference is LOST today (findings)

**F1 — probe outcomes are write-only for routing.** `no_fit` is written by
cloud-probe (`cloud-probe/index.ts:285-287`) but NO consumer reads
`probes[*]` outside `gateway-health.ts` storage (grep: only
`_shared/gateway-health.ts` + `cloud-probe/index.ts` mention `no_fit`).
Result: of 201 ranked candidates, 79 % carry a failing probe mark (76
`no_fit`, 3 `cooling`) yet the overflow candidate list is built from the
catalog alone — each real spill re-tries lanes the prober already knows are
dead, burning the retry budget and the 45 s first-byte timer. **The probe
has no voting rights in candidate order.**

**F2 — `no_fit` conflates "context too small" with "lane dead".** Both the
`limit` branch (context limit learned — that one is USEFUL, it lands in
`limits` too) and the fall-through `else` mark the same `no_fit` with no
reason. `agentrouter/claude-opus-4-8` failing with failCount 727 means the
probe has hammered a dead lane 727 times with no backoff escalation and no
distinction.

**F3 — only ONE fallback in prefs.** `caduceus-prefs.json` `fallback:
["opencode-go/deepseek-v4.1-flash"]`. The router's walk is 1 requested +
1 fallback = 2 candidates — exactly the "all 2 candidate(s) unavailable"
from the incident. The candidate pool has 20 probe-ok lanes; the walk uses
1. The `giveUpMessage` arithmetic (`tried = fallbacks.length + 1`) confirms.

**F4 — probe budget is time-boxed, not value-boxed.** 6/min over 201
candidates = a full sweep takes ~33 min even before failures; failing lanes
without escalating backoff monopolise the budget (F2), so healthy-lane
verification latency (10 min TTL, `OFFICINA_PROBE_VERIFIED_TTL_MS`) can
outrun the sweep on a cold start.

**F5 — sleep-lane idle capacity is untapped for eval.** The dream-eval
harness (rung 1, shipped today) scores endpoints on demand; the sleep lane
(spring 2026-11-21) already runs consolidation on engine slot 1 idle. Free
CLOUD lanes could run the dream EVAL continuously in the background (score
candidates against the frozen eval) and feed a quality ranking back into
`FREE_OWNER_RANK` — turning "free" into "free AND known-good".

**F6 — local idle time is the biggest free bucket.** 79 % of turns are
local, but between sessions the engine idles (cacheWarming streaming,
MEMORY.md). Background work that could run free locally: dream-generator
corpus distillation (S2 of the dream pipeline) on slot 1 during idle —
no cloud tokens at all.

## Direction options (for the owner to rank)

- **O1 — probe-weighted routing (highest leverage)**: consume
  `gateway-health.probes` in overflow candidate ordering — probe-ok lanes
  first, `no_fit`-with-reason demoted, escalating probe backoff
  (failCount-aware). Turns the existing probe investment into actual
  spill quality.
- **O2 — widen the router walk**: prefs fallback list ← top-N probe-ok
  lanes (e.g. 4-6). The "all 2 candidates" class of failure disappears;
  `giveUpMessage` becomes rare.
- **O3 — probe disambiguation**: split `no_fit` into `context_limit`
  (learned, usable for big-context guard) vs `probe_error` (retryable,
  backoff 2^n capped). Saves probe budget, improves mark honesty.
- **O4 — dream-eval quality ranking**: run the frozen eval against probe-ok
  free lanes on a schedule (cloud cost ≈ 0), rank into prefs; also gives
  the dream pipeline its S3 baseline measurement for free.
- **O5 — idle-slot dream generation**: S2 corpus distillation on local slot
  1 idle windows (pairs with the sleep lane), zero cloud spend.
