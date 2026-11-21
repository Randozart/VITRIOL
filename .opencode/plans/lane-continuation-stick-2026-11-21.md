# Lane continuation stick — plan (2026-11-21)

Research: `.opencode/plans/lane-continuation-stick-research-2026-11-21.md`.

## Problem
Both virtual lanes return `{ model: previous }` for every `continuation` turn
(`resilient-default/index.ts:700` lapis, `:1175` nimbus). One local turn then
pins the whole tool loop to local: Nimbus loses its cloud-first priority and
Lapis keeps a contended slot (defeating the 5a7b778 probe).

## Design

One shared, pure predicate — `stickDecision()` in `_shared/lane-stick.ts`:

```
stickDecision({ lane: "lapis"|"nimbus", previousIsLocal, contended, cloudFits })
  -> { stick: true } | { stick: false, reason: string }
```

Rules:
| lane | previous | contended | cloudFits | verdict |
|---|---|---|---|---|
| nimbus | cloud | * | * | stick (cloud priority preserved) |
| nimbus | local | * | true | re-decide (`"nimbus: cloud-first — re-decide off local"`) |
| nimbus | local | * | false | stick (no cloud capacity — a swap would strand the turn) |
| lapis | cloud | * | * | stick (warm cache) |
| lapis | local | false | * | stick (normal tool loop) |
| lapis | local | true | true | re-decide (`"lapis: local slot contended — re-decide"`) |
| lapis | local | true | false | stick (nowehere to go) |

## Stages

**S1** — `_shared/lane-stick.ts` (pure) + `lane-stick.test.ts` covering the
whole table incl. I1 parity cases.
**S2** — wire into both continuation arms in `resilient-default/index.ts`;
contended = `probeBusy || localContended(now)`; cloudFits = `!!candidate`
(Nimbus) / `!!pickCloud(...)` result for lapis — evaluated lazily only when the
predicate needs it.
**S3** — re-decisions publish their reason (band + ledger) so a mid-loop tier
flip is never silent.
**S4** — tests, gates, commit, push.

## Invariants

- **I1 (parity)**: `previous` on the lane's preferred tier ⇒ identical decisions
  to today. Pinned by `lane_stick_parity_matrix` (every combination where the
  verdict is "stick").
- **I2**: Nimbus never serves a continuation on a local model when cloud
  capacity exists and privacy allows. Pinned by `nimbus_leaves_local_when_cloud_available`.
- **I3**: Lapis never serves a continuation on a contended slot when cloud
  capacity exists. Pinned by `lapis_leaves_contended_local`.
- **I4**: a mid-loop re-decision records a reason (band row via `setRouteWidget`
  + ledger `reason`). Pinned by the integration tests asserting the ledger entry.
- **I5**: a stick never counts as a switch for the cache-affinity cap. Pinned by
  `stick_is_not_a_switch` (state unchanged on the stick path).
- **I6**: sensitive/confidential turns never leave local for either lane (the
  predicate must not override the privacy arm). Pinned by
  `privacy_beats_nimbus_redecision`.

## Named tests
- `lane-stick.test.ts`: `lane_stick_parity_matrix`, `nimbus_leaves_local_when_cloud_available`, `lapis_leaves_contended_local`, `nimbus_stays_local_without_cloud`, `privacy_beats_nimbus_redecision`.
- `resilient-default/index.test.ts`: `continuation_off_local_on_nimbus_redecides`, `continuation_on_contended_lapis_redecides`, `continuation_stays_on_local_without_cloud`, `stick_records_no_switch`.
