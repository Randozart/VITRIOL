# The stale-header root cause: pi never tells RPC clients about a model change

**Date:** 2026-11-18 19:30
**Mode:** PLAN (research only — nothing changed)
**Supersedes §1–§2 of** `header-cloud-routed-local-2026-11-18.md` (the retry-exhaustion theory was
incomplete; the mechanism is verified below)
**Symptom (owner):** "the header shows a cloud model, but it tries to route to local"

## 1. Verified causal chain

**Step 1 — pi's model change is an *extension-only* event.**

```js
// dist/core/agent-session.js:2008-2016
async _emitModelSelect(nextModel, previousModel, source) {
  if (modelsAreEqual(previousModel, nextModel)) return;
  await this._extensionRunner.emit({ type: "model_select", model: nextModel, previousModel, source });
}
async setModel(model, options = {}) {
  ...
  this.agent.state.model = model;                       // ← the truth changes here
  this.sessionManager.appendModelChange(...);
  ...
  await this._emitModelSelect(model, previousModel, "set");
}
```

`_extensionRunner.emit(...)` is the **extension** bus. `model_select` occurs exactly **once** in
`agent-session.js` — there is no `_emit({type:"model_changed"})` on the AgentSession event stream, and
`modes/rpc/rpc-mode.js` has **no** bridge for extension events (its only event sink is
`output(toJsonEvent(event))` for AgentSession events).

**Step 2 — so the TUI is never told.** The header's model comes from exactly three places:

| source | file:line |
|---|---|
| `set_model` **response** (the operator's own `/model`) | `tui/src/tui/mod.rs:4136` |
| `get_state` → `model` | `mod.rs:3907`, requested at `:117` (startup), `:700` (after a `handle_response` that returned true), `:3300` (session rehydrate) |
| `model_select` handling | `mod.rs:4534` — **dead code under RPC**, the event never arrives |

There is no per-turn model refresh, so a switch made by an extension leaves the header wrong **until
the session is rehydrated or the operator picks a model again**.

**Step 3 — who switches.** Four call sites, all extension-driven:

| switch | site | when |
|---|---|---|
| to local | `adaptive-router/index.ts:477` | `decideTier` returns `local` (load / privacy / cost / peer arbitration) |
| to local | `adaptive-router/index.ts:646` | `/overflow off` with an active cloud tier |
| to cloud | `adaptive-router/index.ts:452` | tier promotion |
| either | `coupling/index.ts:77` (`llamacpp`) and `:92` (a coupling) | `/model-swap` |

`resilient-default` (lapis/nimbus) is **not** the culprit for a real cloud selection — it only
intercepts its own virtual models; `route` is registered per virtual model
(`pi.registerVirtualModel({provider:"officina", id:"lapis"…})`).

**Step 4 — why it lands on local at all.** `decideTier` is driven by load, privacy, cost and peer
arbitration — **not** by which model the operator selected — and the `manual` latch that is meant to
suppress it has three holes (restore early-return, local pick returns before latching, candidate-gated
adoption), documented in `manual-pick-still-switches-2026-11-18.md`. So one legitimate-looking
demotion (`pi.setModel(localModel)`) leaves the header claiming a cloud model while every subsequent
request is served by `llamacpp/Lapis Occultus`.

## 2. Which defect to fix, and in which order

The **lie** is defect A; the **demotion** is defect B. They are independent — fixing either alone
leaves a bad experience, but A is the one that makes the truth visible, and it is the smaller change.

### A. The header must not be able to go stale (do this first)
`get_state` already carries `model`, and the TUI already parses it (`mod.rs:3907`). Request it at the
two moments a model switch can happen without a `set_model` response:

1. **after `agent_end`** — an extension switches at a turn boundary by design (`adaptive-router`
   comment at `:305`), so once per turn is exactly the right cadence (one cheap RPC round-trip; the
   TUI already issues `get_session_stats` on this path for the ctx row);
2. **after `session_start`** — covers restore/latch paths.

That is a ~10-line change and self-heals against *any* future switcher, including extensions that do
not exist yet. It also makes the dead `model_select` branch (`mod.rs:4534`) honest: keep it (harmless,
and it is the right handler if pi ever forwards the event) with a comment saying it is RPC-inert today.

### B. A manual pick must not be demoted
The three holes from `manual-pick-still-switches-2026-11-18.md`:
1. drop the `event.source === "restore"` early return from the latch path (keep `applying`),
2. latch `manual = true` on an explicit **local** pick too (today `isLocalModel` returns first),
3. replace `shouldAdoptSelectedLane`'s candidate-gated test with "cloud-ish provider" so a resumed
   cloud model that is not in `cloudCandidates` still counts as the operator's choice.

### C. (optional, later) Say which lane served the turn
Once A lands the header is truthful for real models. The remaining honest gap is the **virtual**
models (`officina/lapis`, `officina/nimbus`), where one session legitimately spans lanes per request —
there the header should read `Lapis Occultus → local` / `→ glm-5.2` from `lastDecision`, which
`resilient-default` already publishes to the routing registry.

## 3. Invariants

- **I1 — header truth:** after any turn completes, the header's model equals pi's `state.model`.
  Pinned by a Rust test that feeds `agent_end` → asserts a `get_state` request is emitted, and by a
  state test that a `get_state` response changes the rendered label.
- **I2 — no manual demotion:** with `manual` latched, `decideTier` is not consulted; pinned by the
  three router tests in the sibling plan.
- **I3 — parity when unmanaged:** sessions that never switch models issue no extra RPC beyond the
  existing `get_session_stats` call at `agent_end` (asserted by request-count in a test).
- **I4 — virtual lanes unchanged:** lapis/nimbus routing bytes-identical (C is additive only).

## 4. Files

- `officina/tui/src/tui/mod.rs` — `get_state` after `agent_end`/`session_start`; comment the
  RPC-inert `model_select` branch.
- `officina/.pi/extensions/adaptive-router/index.ts` — the three latch holes.
- Tests: `tui/src/tui/mod.rs` (or the existing response-handling test module) + `adaptive-router`.

## 5. How the owner can confirm it right now, before any code changes

The **route strip / `/routes`** reads the routing registry and the ledger, which record the *served*
lane per request; the **header** reads `set_model`/`get_state`. If the header says
`GLM 5.2 · caduceus` while `/routes` shows `llamacpp/Lapis Occultus`, that is defect A demonstrated
live — no code needed to prove it.
