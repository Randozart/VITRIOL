# Officina consolidation — ONE plan, GRIND-ready

**Date:** 2026-11-18 21:20
**Merged from (superseded by this file):**
`officina-de-spaghetti-2026-11-18.md` · `multi-session-in-one-project-2026-11-18.md` ·
`next-row-under-route-2026-11-18.md` · `manual-pick-still-switches-2026-11-18.md` (closed, `e99c981`) ·
`model-header-stale-2026-11-18.md` (closed, `e99c981`) ·
`aetherskiff-opaque-2026-11-18.md` (closed, opaque kitty/alacritty + generator default)

---

## 0. Goal

Turn Officina's **implicit coordination channels** into **contracted ones**, and make the system
survive the way it is actually used (3–4 sessions on one engine). Not a rewrite: 1,498 vitest + 381
cargo tests are the safety net, and every stage below ends green.

**Non-goals (decided, do not revisit in GRIND):**
- No framework. A framework would be a fourth untyped channel.
- No feature deletion. Routing sprawl is *consolidated*, not removed.
- `parallel = 1` stays. `parallel = 2` would halve each slot to 131k and trade away native context.
- No new cross-extension imports, ever (stage 6 enforces it).

**Baseline to move (measured 2026-11-18):** `pi.setModel` callers **9 → 1** · band priority map in
`layout.rs` → **one registry** · cross-extension imports **10 → 0** · unregistered `globalThis` **56 → 0**
· `OFFICINA_*` knobs **160 → one declared module** · TUI monoliths `state.rs 9,595 / mod.rs 5,781 /
layout.rs 5,604` → **under 2,500 each**.

---

## 1. Stages (each is a commit, each ends green)

Gates for **every** stage: `npx vitest run` (from `officina/`), `cargo test` (from `officina/tui/`),
`npx tsc --noEmit`, `cargo fmt --check`, Praetor clean, CI green. Any Rust change that ships visible
behaviour ⇒ **rebuild + `cp` to `~/.local/bin/officina`** before the stage is done.

### S1 — Cross-session hygiene (tiny, no dependencies)
1. **Delete the phantom aggregate** `~/.vitriol/officina/routing-registry.json` (two dead peers from
   2026-11-11); add a test that the shard dir is the **only** source (`readLivePeers` already reads
   shards only — pin it).
2. **Session-key the compaction stamp**: `_shared/compact-on-finish.ts` writes one global
   `compact-on-finish.json`, so session A silences session B for the whole cooldown. Move to
   `~/.vitriol/officina/compact-on-finish/<sessionId>.json`; keep the one-shot guarantee **per
   session**. Test: two session ids, independent stamps.
3. **Shared-state inventory test**: enumerate `~/.vitriol/officina/*`, classify each file
   `session-keyed` or `last-writer-wins`, and fail when a **new** shared file appears unclassified.

*Invariants:* I1 stamps are per session (two-writer test). I2 the inventory test fails on a new
unclassified file (negative control), passes on today's set.

### S2 — Band registry + the "next under route" fix (visible win)
The strip is 3 rows for 16 writers, ordered by a `match` in `layout.rs` naming 5 keys; everything else
falls to `_ => 9`, which is how `▸ next:` lands under `route manual`.
1. New `_shared/band.ts`, modelled on the proven `_shared/sidebar.ts`: extensions **register rows**
   `{id, tier, render}`; tiers `state` | `progress` | `suggestion`; `globalThis`-backed (jiti rule) and
   registered in `jiti-isolation.test.ts`.
2. TUI renders the **declared** list (`layout.rs`), not a hardcoded `match`; transfer all 16 writers:
   `overflow-route`, `vitriol-decode`, `plan-mode`, `deep-research-mode`, `phase-model` = `state`;
   `composer-prediction` = `suggestion`; `agent-mode`/`engine-fire` stay out of the band.
3. Rule: **a suggestion row never renders while any state row is present.** Cap stays 3;
   `stabilize_band_parts` untouched (the ctx row must not move — the 2026-11-16 flicker fix).

*Invariants:* I1 no suggestion while a state row exists (behaviour test, plus the no-state-row case as
its negative control). I2 cap 3, ctx row unmoved. I3 an **unknown** key still renders (the rule keys on
the suggestion widget, not on "unmatched", or a future extension vanishes).

### S3 — Session capacity (the dominant multi-session cost)
Measured: 4 sessions × ~1.7 GB state ≈ 6.7 GB against `cache_ram = 8192`; a miss costs a full
re-prefill at ~209 tok/s (~18 min at 225k).
1. `[server] cache_ram` 8 → **16 GB** (fingerprint `cram=`), restart, verify `/cache`.
2. `/cache` reports `states ≈ cap / state_size` and the live session count; warn when
   `sessions × state ≈ cap`.
3. **Live probe** that peer demotion actually fires: two sessions, `freeSlots == 0`, one demoted with a
   reason naming its rank — the machinery is only unit-tested today.

*Invariants:* I1 with N=1, decisions byte-identical to today (golden). I2 the demotion probe fails if
the peer arm is disabled (negative control). I3 the arithmetic is pinned against a synthetic cap.

### S4 — One routing authority (highest leverage: kills a whole bug class)
Nine places can change the model, each with its own latch — every "it randomly switched" report was two
of them disagreeing (three of them were fixed by hand on 2026-11-18).
1. `_shared/routing/decision.ts` — **pure** `(facts) => {target, reason, owner}`; no I/O.
2. `_shared/routing/apply.ts` — the **only** `pi.setModel` caller; records intent
   (`_shared/model-switch-intent.ts` becomes internal), cools, publishes to the registry.
3. `_shared/routing/facts.ts` — engine load, peers, health, pins, budget, phase.
4. Convert `adaptive-router`, `resilient-default` (lapis/nimbus), `phase-model`, `coupling`,
   `auto-route`, `hang-watchdog` into **policy inputs** (a pin, a phase mapping, an escalation).
   `llama-cpp-provider`'s startup sync becomes a declared policy, not a marker hack.

*Invariants:* I1 a manual pin is never overridden — **one** latch, one test. I2 virtual lanes keep
per-request semantics. I3 no policy inputs ⇒ byte-identical decisions to today (golden matrix, the
`overflow-peer.test.ts` pattern). I4 `pi.setModel` appears in exactly one file (source scan).

### S5 — Cross-session file claims (advisory, never blocking)
1. **Prerequisite bug:** `_shared/active-files.ts` is a **module-level** Map commented "one per
   process, shared across all extensions" — false under jiti isolation, so the tool-result-clearer's
   protection is unreliable **today**. Make it `globalThis`-backed; register in `jiti-isolation.test.ts`.
2. Claim registry: `~/.vitriol/officina/claims/<pid>.json` `{pid, sessionId, paths[], at}`, written on
   edit/write, TTL 10 min, same shard/GC pattern as routing.
3. Warning on a path claimed by another **live** pid — one row in the S2 band (`state` tier), plus
   `/claims`. **Never** refuse a write; a claim read failure degrades to silence.

*Invariants:* I1 claims never block (a write under a claim succeeds — negative control). I2 no
collision across pids. I3 N=1 is silent.

### S6 — Enforce the boundary
1. A test that **fails** when an extension imports another extension (only `_shared/*` and its own dir).
2. Migrate the 10 current violators into `_shared/` (e.g. `phase-model` → `../llama-cpp-provider/config.ts`).
3. `_shared/config.ts`: every `OFFICINA_*` knob declared with default + doc; unknown names warn. Parity
   table test (each knob keeps its default and precedence).

*Invariants:* I1 zero cross-extension imports. I2 knob parity. I3 a new violation fails CI.

### S7 — Decompose the three monoliths, with a ratchet (behaviour-neutral)
`mod.rs` → `wire/` `responses/` `events/` `commands/` · `layout.rs` → `band` `composer` `transcript`
`header` · `state.rs` → `state/` by concern. Ratchet test: no `.rs` file may exceed its recorded
budget; new behaviour lands in a module.

*Invariants:* rendering byte-identical — reuse the existing `render-invariants` module (buffer bounds,
idempotence, ctx-row stability). Tests only; no behaviour change.

### S8 — Delete the dead surface
Remove the deprecated JS frontend (`officina/officina.mjs`, `runtime/patched/*`, build-patch canaries).
Its "patch only in the generated file" incidents go with it.

*Invariant:* nothing in the Rust TUI imports from `officina/runtime/` (source scan).

---

## 2. Order and why

```
S1 hygiene ─┐
S2 band ────┼─→ S5 claims (needs the band row)
S3 capacity ┘
S4 routing authority   (independent, biggest win, do once S1–S3 are green)
S6 boundary            (after S4: fewer violators to migrate)
S7 decompose           (after S2/S5 so files move once)
S8 delete surface      (any time after S6)
```

S1–S3 are one session's work and remove three silent failure modes. S4 is the structural win. S6/S7
are the guards that stop recurrence.

## 3. GRIND task list (copy into `update_tasks`)

1. S1 — delete phantom registry aggregate + pin shards-only read
2. S1 — session-key the compaction stamp (+ two-writer test)
3. S1 — shared-state inventory test with negative control
4. S2 — `_shared/band.ts` registry (jiti-registered) + migrate 16 writers
5. S2 — suggestion-yields-to-state rule + behaviour tests; TUI band renders declared list
6. S3 — `cache_ram` 16 GB + `/cache` states row + live demotion probe
7. S4 — `_shared/routing/{decision,apply,facts}`; `pi.setModel` in one file
8. S4 — convert six routers to policy inputs; pin semantics + golden matrix
9. S5 — fix `active-files` jiti bug; add claim registry
10. S5 — advisory claim warning row + `/claims`
11. S6 — boundary test + migrate 10 violations
12. S6 — `_shared/config.ts` knob registry with parity table
13. S7 — decompose `mod.rs`/`layout.rs`/`state.rs` + file-size ratchet
14. S8 — delete the legacy JS frontend + source-scan guard
15. Final — all gates green, TUI rebuilt+installed, CI green, plans cross-linked

## 4. Definition of done

- `pi.setModel` in **one** file; band priority in **one** registry; no cross-extension imports; no
  unregistered `globalThis`; no `.rs` file over budget; every `OFFICINA_*` knob declared once.
- Multi-session: stamps per session, claims visible, `/cache` states the capacity arithmetic.
- The `▸ next` row never appears under `route …`.
- `1,498+ vitest`, `381+ cargo`, `tsc`, `fmt`, Praetor, CI all green; installed TUI md5 recorded.

## 5. Risk and rollback

- **S4 is the only stage that can strand a session on a bad model.** Mitigation: land `decision.ts`
  pure + golden matrix **first** (behaviour identical, no wiring change), then switch call sites one
  extension at a time, each with its own commit; `/overflow on` remains the operator's escape hatch.
- **S3 changes a live engine flag** — revert by setting `cache_ram` back; the fingerprint line shows
  `cram=` so drift is visible.
- **S7 touches everything and changes nothing** — it is pure file motion; if a gate goes red, revert the
  stage and keep S1–S6, which are independent.
- Every stage is a separate commit on `main`, so any stage can be reverted without unpicking the rest.
