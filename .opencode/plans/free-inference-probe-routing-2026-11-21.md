# Free-inference leaks — plan (2026-11-21)

Probe-weighted routing (O1), no_fit disambiguation + probe backoff (O3),
wider router walk (O2), dream-eval lanes mode (O4, thin slice). Research:
`free-inference-research-2026-11-21.md`. Confirmed by owner (PIPELINE
confirm gate, "Confirmed").

## Changes

- **S1 `_shared/probe-order.ts` (pure)**: `orderCandidates(candidates,
  probeFacts, ctxTokens)` — probe-ok first (ascending latencyMs), then
  unprobed (original order), then `context_limit`-marked lanes whose
  learned limit fits the request, then failing/cooling lanes last.
  adaptive-router's `discoverFreeModels` consumer joins marks via
  `readGatewayHealth().probes`.
- **S2 probe kinds**: `no_fit` split into `context_limit` (also lands a
  `limits` entry — lane stays usable with a fit gate) vs `probe_error`
  (failCount-based backoff: `min(2^failCount, 8) * baseInterval` capped).
  cloud-probe `index.ts` limit/else branches; `gateway-health.ts`
  ProbeKind union + migration of existing `no_fit` → `probe_error` on
  read (old marks degrade to retryable, not dead).
- **S3 fallback depth**: prefs gain `fallback_depth` (default 4). The
  caduceus extension, when writing prefs or on doctor, offers seeding the
  fallback list with the top-N probe-ok lanes. Router walk already
  iterates lists — no router change.
- **S4 `/dream-eval --lanes [N]`**: scores the top-N probe-ok free lanes
  against the frozen eval set; report lists per-lane pass rates. No
  auto-promotion into prefs.

## Invariants

- **I1 — parity**: with an empty/absent probe map, `orderCandidates`
  returns candidates in the original order. Pinned by:
  `probe-order.test.ts > unprobed pool keeps catalog order`.
- **I2 — ok-first**: every probe-ok lane precedes every lane with a
  failing mark. Pinned by: `probe-order.test.ts > probe-ok lanes lead`.
- **I3 — context_limit is usable**: a lane marked `context_limit` with
  learned limit ≥ ctxTokens stays in the ordered list (ranked above
  `probe_error`). Pinned by: `probe-order.test.ts > context-limited lane
  stays eligible when it fits`.
- **I4 — backoff is bounded and never blocks ok**: backoff factor
  ≤ 8×, and an `ok` probe always resets it. Pinned by:
  `probe-scheduler.test.ts > failing probe backoff escalates and caps`,
  `ok probe resets backoff`.
- **I5 — walk depth is prefs-driven**: missing `fallback_depth` →
  behaviour identical to today. Pinned by:
  `fallback-depth.test.ts > missing pref keeps current fallback list`.
- **I6 — lanes mode is read-only**: `/dream-eval --lanes` never writes
  prefs or the eval set; only reports. Pinned by:
  `dream-eval-lanes.test.ts > lanes mode does not persist prefs`.

## Named tests

1. `_shared/probe-order.test.ts`: I1/I2/I3.
2. `_shared/gateway-health.test.ts` additions: kind migration (old
   `no_fit` → `probe_error`), limit learning on `context_limit`.
3. `_shared/probe-scheduler.test.ts` additions: I4.
4. `dream-eval-lanes.test.ts`: I6 + ordering of scored lanes.
5. Existing suites must stay green (parity by construction).
