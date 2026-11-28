# VITRIOL handoff — 2026-11-28 12:36 +01:00 Europe/Amsterdam

## Start here

Read the current AGENTS.md, then inspect `git status` before making changes.
The user wants local coding experiments to produce useful Rust port code along
the way. The last implementation stage is complete. The natural continuation
is a small Rust-core/RPC integration step with JavaScript parity checks.
A separate untracked gateway proposal is present; distinguish it from completed
work and confirm which thread the user wants to continue.

Use work-plans before edits, verified-development for implementation gates and
project-resumption for checkpoints. Preserve uncommitted work. Commit completed
stages separately. No publication or production router switch was performed.

## Repository at this handoff

Branch: `main`. Implementation HEAD before this documentation handoff:
`73b75106925f7cf60961af932eb2d1fd4fa41d95`.

| Commit | Delivered work |
| --- | --- |
| `7335b31` | Caduceus candidate ordering and retry policy in Rust core |
| `73b7510` | Park caps, pruning and give-up reporting; trial report and docs |
| `5389c2a` | TTFT/checkpoint I/O telemetry and paired benchmark tooling |
| `7a6852b` | Explicit telemetry verification findings |
| `d849a92` | Completed-request dashboard telemetry |

The tracked tree matched `73b7510` when checked. The pre-existing untracked file
`.opencode/plans/2026-11-28-094849-codex-caduceus-gateway.md` was preserved and
excluded from this handoff commit. It proposes an opt-in Codex/Caduceus endpoint
and records feasibility findings. It does not establish a completed authenticated
Codex tool round-trip, cost comparison or production configuration change.

## Rust port that actually exists

[vitriol-core/src/caduceus.rs](../../vitriol-core/src/caduceus.rs) exports:

- `candidate_order`: ordered, trimmed, deduplicated candidates excluding future parks.
- `should_retry`: exact retry statuses, with forwarded output preventing retries.
- `capped_park_ms`: bounded park hints, including the source's numeric edge behavior.
- `prune_parked`: immutable future-park/reason filtering, excluding the phantom `?` lane.
- `give_up_message`: compatible earliest-lane reporting and rounding.

The existing `vitriol-core` crate was extended, rather than creating another
scratch-only library. The live Caduceus router still uses JavaScript. These new
helpers are available to Rust library callers, not yet exposed through RPC or
wired into production routing. Existing RPC methods remain unchanged.

[The core README](../../vitriol-core/README.md#caduceus-policy) defines the typed
input domain and compatibility details. Important edges include ECMAScript trim
(U+FEFF/U+2028/U+2029 included; U+0085/U+180E excluded), NaN cap propagation,
forwarding precedence and the source helper's zero-timestamp truthiness quirk.
`None` cap disables the cap; callers wanting the normal ceiling pass
`Some(DEFAULT_MAX_PARK_MS)`.

## How the model-assisted trial worked

Kintsugi used the installed Rust Officina TUI and actual read/write/edit tools.
A scratch checkout ran under bwrap with a read-only root, private HOME/catalog,
no shell tool, and read-only reference/tests/manifests. The evaluator supplied
specifications, independent JS-generated fixtures and compiler/parity/clippy
feedback. The model wrote the implementation. Mechanical rustfmt of the final
raw model file exactly matches the integrated Rust module.

There were two implementation turns and four repair/cleanup turns. First-pass
failures were retained: map-key typing, Unicode trimming, NaN-cap behavior and
clippy issues. Do not describe this as an unassisted first-pass success.

Across 20 local model requests, the session reached **24,840 filled tokens** and
35.02 tok/s median decode. Active coding turns totaled 310.08 seconds, excluding
evaluator pauses. Median proxy-observed first meaningful text/tool delta was
1.875 seconds. This is one medium-context workflow, not a new 200K trial, a 70
tok/s result or a model-wide quality comparison. LLM text may vary; deterministic
behavior applies to the control-plane API contracts and independent tests.

Full report: [model-assisted Rust port](2026-11-27-125306-model-assisted-rust-caduceus-port.md).
Private scripts, raw model files, receipts, PTY captures and recovery evidence:
`/tmp/vitriol-rust-port-20261127/`.

## Verification and its freshness

At the implementation freeze:

- 36 Cargo tests passed, including the existing suite and five independent policy tests.
- 30 original JavaScript policy tests passed under Vitest.
- Rust formatting, clippy with `-D warnings`, and the release build passed.
- Oracle regeneration was byte-stable: 96 ordering cases, 600 status cases with
  forwarding/timeout combinations, 35 cap cases, three prune cases and nine
  reporting cases, plus explicit native numeric boundaries.
- Scoped Praetor on core source had zero blocking findings. This is a scan,
  not a proof of every behavior; test coverage supplies separate evidence.
- All eight changed-scope repository gates passed.

Historical evidence:
`/tmp/vitriol-rust-port-20261127/verification/20261127T121619Z-b9c57c29/evidence.json`.
Its original record says passed. On resumption, the old checkpoint/evidence
freshness is stale because the working snapshot now includes later draft/docs
and the environment may have changed. The tested Rust/telemetry source paths
had no diff from `73b7510` during this handoff check. Do not present yesterday's
whole-snapshot evidence as a fresh verification of new work.

Useful commands from the repository root:

```sh
cargo test --locked --offline --manifest-path vitriol-core/Cargo.toml
cargo fmt --manifest-path vitriol-core/Cargo.toml --check
cargo clippy --locked --offline --manifest-path vitriol-core/Cargo.toml --all-targets -- -D warnings
(cd officina && ./node_modules/.bin/vitest run scripts/caduceus-router-lib.test.mjs)
node vitriol-core/tests/generate_caduceus_vectors.mjs
```

The generator writes the checked-in fixture. Run it before the final freeze and
inspect changes. `node --test` is the wrong runner for the Vitest test file.

## Live runtime: recheck before any model experiment

Read-only observation at 2026-11-28 12:36 +01:00:

- `vitriol-server.service`: active, PID 816552; `/health` returned `ok`.
- `vitriol-autosave.service`: active, PID 1218861.
- Slot 0: idle, task 301528, window 229,376, **158,408 filled tokens**.

The established daily driver is the reference Kintsugi GPU1 setup, selected as
`kintsugi-reference-gpu1`, serving Lapis through the existing local integration.
Yesterday's coding trial restored its then-current **1,512-token** slot. The
current 158,408-token state demonstrates subsequent use. **Do not restore the
old trial checkpoint over today's conversation.** Idle at this one observation
is not an ongoing maintenance window. Take a fresh owner-aware checkpoint for
any newly authorized state-changing experiment. Do not kill the active engine
as though it were a stale test process.

No engine restart or profile change was needed for the Rust trial. Its Officina
process and timing proxy were stopped. The uniquely named baseline snapshot
was retained in the engine checkpoint directory for recovery; it is historical.
The old scratch provider/proxy settings must not become production defaults.

## Telemetry work and open limits

[Metric definitions](../../docs/inference-telemetry.md) and
[the implementation plan](2026-11-27-092725-inference-dashboard-telemetry.md)
record completed-request timings, observed cached tokens, draft acceptance,
native TTFT means where available, and PID/port-scoped checkpoint operation I/O.
The installed release was rebuilt. Autosave alone was reloaded. The journal
reader was fixed to include user-service stdout rather than only system journal
lifecycle messages. Checkpoint I/O was observed in the actual dashboard.

Reference Kintsugi lacks the in-tree native TTFT series. Stream clients/proxies
can measure first-content latency separately. Checkpoint save/restore duration
includes serialization/file I/O and can include draft state; its byte count can
exclude draft sidecars. It is not GPU-only DMA or continuous tier-transfer timing.

A final 16-pair isolated 0.5B GPU0 benchmark had a +0.892% upper 95% bootstrap bound
on decode-time overhead. This is limited by its workload, display activity and
sample variability. It does not certify overhead at 200K Kintsugi depth.

Telemetry tests passed 72 Rust and seven Python tests, but its final generic
verification retained two failing gates: 20 pre-existing autosave Ruff findings
and three audited Bandit URL-scheme warnings (two old, one in the benchmark).
Do not claim all telemetry gates passed or suppress those findings.
Historical record:
`/tmp/vitriol-telemetry-stage3/verification/20261127T113701Z-72196f84/evidence.json`.
All `/tmp` artifacts were present at handoff but may disappear later.

## Suggested continuation

For the Rust-port thread, inspect the existing core RPC dispatcher and client
before adding an adapter. A bounded next stage is typed exposure of these
helpers, with strict input validation and JS/Rust shadow parity before any live
router switchover. Keep the port useful and incremental. Preserve the distinction
between model-authored code, evaluator fixes and independently verified outcomes.

The broader research plans for cache continuity, streamed KV attention, adaptive
speculation and selective historical KV remain separate open work. Read their
existing plans before changing engines. For performance differences, inventory
and hot-file diff the reference source before profiling this fork. Native replay
experiments have historical bounds/rollback failures; do not enable them based
on the old speed headline alone.

If the user instead wants the Codex gateway thread, inspect the preserved draft
and its stated verification gaps first. This handoff authorizes no provider,
credential, routing-default or backend-service change.
