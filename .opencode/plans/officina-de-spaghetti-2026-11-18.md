# Officina is spaghetti: measured, and what to do about it

**Date:** 2026-11-18 20:40
**Mode:** PLAN (research only — nothing changed)
**Owner:** "We really need to make Officina less spaghetti coded."

Agreed. Below is the measurement first, then the *structural* cause (which is not sloppiness), then
five moves ordered by leverage. No rewrite proposed: the 1,498-test suite is the asset that makes
staged surgery safe.

---

## 1. The shape, measured

| surface | number |
|---|---|
| TUI crate (`tui/src`) | **33,861 LOC** — `state.rs` 9,595 · `mod.rs` 5,781 · `layout.rs` 5,604 (63 % of the crate in three files) |
| Extensions | **63 dirs**, **30,477 LOC** non-test TS |
| Files that call `pi.setModel()` | **8** (`adaptive-router`, `auto-route`, `coupling`, `phase-model`, `hang-watchdog`, `llama-cpp-provider`, + the `model-switch-intent` helper) plus `resilient-default`'s virtual `route` closure = **9 switching authorities** |
| Extensions writing widgets/status | **16**, into a band with **3 rows** and a priority map that names **5** keys |
| `pi.on(...)` handlers | `session_start` **32** · `tool_result` **19** · `before_agent_start` **13** · `context` **13** · `model_select` **7** |
| Cross-extension imports (A imports B, bypassing `_shared`) | **10 extensions** (e.g. `phase-model` → `../llama-cpp-provider/config.ts`) |
| `globalThis` singletons | **56** uses |
| `OFFICINA_*` env knobs | **160** distinct names |

Each number on its own is defensible. Together they describe a system where **coordination is
implicit**: many writers, unordered event fan-in, string-keyed channels.

## 2. Root cause: pi's extension model has no shared runtime

pi loads **every** extension through its own jiti instance with `moduleCache: false` (documented in
AGENTS.md, pinned by `_shared/jiti-isolation.test.ts`). There is no dependency injection, no service
registry, no ordering between extensions. So any coordination *must* happen through:

1. `globalThis` (56 places) — untestable in isolation, invisible in types;
2. string-keyed UI channels (`setWidget("overflow-route", …)`, `setStatus("overflow", …)`) — no
   namespace, no owner, no priority contract;
3. `process.env` (160 knobs) — read at arbitrary points, no single schema.

**This is the substrate the owner is feeling.** Two of the last four bugs I fixed were *not* logic
errors; they were two extensions disagreeing through these channels (three routers latching a pin;
ten band writers competing for three rows). Local cleanliness cannot fix a channel that has no
contract.

## 3. Five moves, ordered by leverage

### M1 — One routing authority (highest leverage, ~kills a bug class)
Today nine places can change the model, each with its own latch, its own cooldown and its own idea of
"manual". Every "it randomly switched" report has been a disagreement between two of them.

Target: `_shared/routing/` owns the decision; extensions **declare policy** instead of switching.

```
_shared/routing/decision.ts   pure: (facts) => {target, reason, owner}   ← unit-testable, no I/O
_shared/routing/apply.ts      the ONLY caller of pi.setModel (records intent, cools, publishes)
_shared/routing/facts.ts      engine load, peers, health, pins, budget, phase
```
`adaptive-router`, `resilient-default` (lapis/nimbus), `phase-model`, `coupling`, `auto-route` become
policy inputs: a pin, a phase mapping, an escalation. `_shared/model-switch-intent.ts` and the eight
latch sites collapse into one.

**Invariants:** I1 a manual pin is never overridden (single latch, one test); I2 virtual lanes keep
their per-request semantics; I3 with no policy inputs, the decision is byte-identical to today
(golden matrix — the pattern `overflow-peer.test.ts` already uses).

### M2 — One band registry (fixes "next under route" structurally)
Mirror `_shared/sidebar.ts` (which already solved this for the sidebar) for the composer band:
extensions **register rows** `{id, priority, render()}`; the TUI renders the declared list, not a
hardcoded key map. Priority becomes a documented tier (`state` / `progress` / `suggestion`), and the
"suggestion yields to state" rule lives in one place instead of in `layout.rs`'s `match`.

**Invariants:** suggestion rows never render while a state row exists; the ctx row never moves
(band settling untouched); an unknown row still renders (no silent drops).

### M3 — Decompose the three monoliths, with a ratchet
`mod.rs` (5,781) → `wire/` (loop, backoff, respawn) · `responses/` · `events/` · `commands/`.
`layout.rs` (5,604) → `band.rs` · `composer.rs` · `transcript.rs` · `header.rs`.
`state.rs` (9,595) → `state/` split by concern (chat, tools, dialogs, recovery, theme).

Gate: a file-size ratchet test — no `.rs` file may exceed its recorded budget, and new behaviour must
land in a module. I just added `include_str!("mod.rs")` source scans to test the wire loop, which is
itself a symptom: that wiring is untestable because the loop is not a unit.

**Invariants:** rendering output byte-identical (the render-invariants module already asserts buffer
bounds, idempotence, ctx-row stability — reuse it as the safety net); no behaviour change in this
move, tests only.

### M4 — Enforce the boundary
- A test that **fails** when an extension imports another extension (only `_shared/*` and its own
  dir), with the 10 current violators migrated into `_shared/` first.
- Every `globalThis` singleton must be declared in a registry and covered by `jiti-isolation.test.ts`
  — 56 sites, so add them incrementally, starting with the ones read by more than one extension.
- One typed config reader (`_shared/config.ts`) for the 160 knobs: declared names, defaults, docs.
  Retire aliases; unknown `OFFICINA_*` names get a warning.

**Invariants:** parity — each knob keeps its current default and precedence, pinned by a table test.

### M5 — Delete surface
The JS frontend (`officina/officina.mjs`, `runtime/patched/*`, pi bundle hooks) is **already
deprecated** in AGENTS.md; its build-patch canaries keep a whole subsystem alive. Deleting it removes
code no one runs and one entire class of "patch only in the generated file" incidents.

## 4. Sequencing and cost

| stage | scope | risk | payoff |
|---|---|---|---|
| **S0** | M2 (band registry) + the "next" fix already planned | low | one visible bug dies, and the pattern is proven on a small surface |
| **S1** | M1 (routing authority) | medium — touches live routing | kills the "randomly switches" class; 9 → 1 latch |
| **S2** | M3 (decompose) with the ratchet, behaviour-neutral | low (tests only) | the monoliths stop growing |
| **S3** | M4 (boundary tests + config) | low | prevents re-spaghetti |
| **S4** | M5 (delete the legacy frontend) | low | less surface |

S2 can run in parallel with S1 (different files). Each stage is a commit; each has its invariants
above; each ends green on `1481+ vitest / 381 cargo / tsc / fmt`.

## 5. What I am *not* proposing

- **Not a rewrite.** 1,498 tests encode behaviour we cannot afford to rediscover.
- **Not "add a framework".** The missing piece is *contracts on the channels we already have*
  (globals, widget keys, env) — a framework would be a fourth channel.
- **Not deleting features.** The routing sprawl is consolidated, not removed: each extension keeps its
  policy, only the *actuation* becomes single-owner.

## 6. Definition of done (measurable)

- `pi.setModel` appears in **one** file.
- `belowEditor` priority appears in **one** file (a registry), not in a `match` in `layout.rs`.
- No extension imports another extension (test-enforced).
- Every `globalThis` singleton is registered and isolation-tested.
- No file over budget; the three monoliths under ~2,500 lines each.
- `OFFICINA_*` knobs enumerated in one module with defaults and docs.
