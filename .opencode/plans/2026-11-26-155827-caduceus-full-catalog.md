---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T15:58:27+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "0fa3f41f992b546839116d8b04ed500d053c36fe"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-153957-caduceus-dashboard.md"]
---

# caduceus-full-catalog

## Purpose

Show every model advertised in Caduceus's catalog in the VITRIOL dashboard pane.

## Approach and alternatives

Keep live request and park telemetry, merge it with the existing local catalog, and add search and explicit catalog refresh using the established sync command.

- Querying the gateway from Rust would duplicate Caduceus authentication and provider filtering. The existing shared catalog keeps the same enumeration across clients, including aliases.

## Scope and documentation

VITRIOL TUI and operations guide. Keep the full catalog available if the router is down. No routing config or Officina changes.

## Invariants

1. Every distinct Caduceus-advertised ID is listed by default. Match search case-insensitively against the exact IDs while keeping parked and latest-request routes available. Pin with cache, navigation and render fixtures.
2. Router eligibility and cooldown counts depend only on live request candidates. Catalog-only IDs must never appear eligible or become targets for Unpark. Pin with merged and unreachable snapshots.
3. The catalog remains browsable during a router outage. The canonical sync command is explicit and asynchronous. No refresh may modify the routing policy or send inference traffic.
4. Selection, exact route log scope and unpark use the same stable model ID even as the catalog refreshes. Exercise with overlapping names and an unrelated parked lane.

## Verification

- Cargo fmt, test, build and Clippy, terminal fixture with more models than the visible list, live release check against the user's catalog, Praetor comparison and changed-scope evidence under /tmp/vitriol-caduceus-catalog.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T15:58:27+01:00 — Plan created before implementation.

- 2026-11-26T16:03:29+01:00 — Release and formatting passed, with 58 Rust tests and no new Praetor signatures. Terminal fixture verified 40 IDs during a router outage, case-insensitive off-screen search, canonical sync dispatch, and picking up the 41st ID. The live release displayed all 416 distinct cached IDs and found one beyond the viewport. Final evidence is under /tmp/vitriol-caduceus-catalog/verification/.
