# Second Officina session → cloud routing (2026-11-10)

Owner ask: "run a second officina session which the router knows to route
elsewhere." The daily-driver engine is `--parallel 1` (one slot, 262K ctx),
so a second local session cannot be served by the stone. The existing
routing machinery (adaptive-router + Caduceus free catalog + budget) is
exactly the right vehicle — this is a configuration gap, not a missing
feature.

## What already exists (verified in code)

- `adaptive-router/index.ts`: arms on `OFFICINA_OVERFLOW=1`
  (currently in `~/.vitriol/officina/env`), kills on `OFFICINA_NO_OVERFLOW=1`
  (NOT currently set). Decides at every `before_agent_start` turn boundary.
- `overflow.ts` policy: `decideTier` — local busy → cloud spill
  (maxSwitches=2, dwell, return budget), local DOWN → emergency cloud
  (failover, no switch cap), `privacy != safe` → local only.
- `OFFICINA_OVERFLOW_LOCAL` env: overrides the local model spec (line 85
  of overflow.ts) — i.e. "local" for a given TUI process can be redefined.
- `tui/src/envfile.rs`: `load_officina_env()` applies
  `~/.vitriol/officina/env` with shell-override-wins semantics
  (`??=` parity) — so a per-launch shell export wins over the env file for
  that one TUI process.
- `pickCloud` filters the Caduceus catalog for tool-capable zero-priced
  lanes respecting budget + cooldowns.
- Privacy gate: `classifyTurn` — confidential prompts never leave local.

## Gap analysis

The router is per-TUI-process (each TUI spawns its own `pi --mode rpc`
child with its own extension instances — jiti isolation). Both TUIs read
the same env file. So today a second TUI:

1. Spawns, reads the same env, gets the same `officina/lapis` default.
2. adaptive-router samples the SAME engine; if the first session is idle,
   the second session gets tier=local and tries the same single slot →
   queues behind the first (or errors if the slot is mid-turn).

There is no per-session identity in the routing decision. The env file is
the only per-launch override surface.

## Options

### Option A — launch-time env override (no code changes)
For the second TUI, export `OFFICINA_NO_OVERFLOW=1` so adaptive-router
disarms, and set the default model to a cloud candidate via the model
picker or `--model` CLI. This is the *existing* "manual model selection
pauses routing" path — but it's manual, not automatic.

**Verdict**: works today, but requires the operator to remember to pick a
cloud model. Not "the router knows to route elsewhere."

### Option B — `OFFICINA_OVERFLOW_LOCAL` override (no code changes)
Export `OFFICINA_OVERFLOW_LOCAL=caduceus/opencode-go/deepseek-v4.1-flash`
for the second TUI. adaptive-router then treats that cloud model as
"local" and never spills — the second session is pinned to the cloud
candidate, bypassing the busy/dwell/switch logic entirely.

**Verdict**: simplest automatic route. No code, no new env key. The second
session is a cloud session from turn 1. Privacy still enforced (the
`privacy != safe` check still runs, but "local" is now the cloud model —
which is a privacy leak for confidential prompts). **Needs a privacy
note**: this overrides the local-only guarantee.

### Option C — new env key `OFFICINA_SESSION_TIER=cloud` (small code change)
Add a one-line check in `adaptive-router/index.ts`: if
`process.env.OFFICINA_SESSION_TIER === "cloud"`, force `tier = "cloud"` at
boot and skip the local-resolve path. Privacy gate still runs (confidential
prompts stay local — but if "local" is the busy single slot, they queue;
an explicit privacy override `OFFICINA_SESSION_PRIVACY=local-only` could
refuse to start on cloud).

**Verdict**: most explicit, smallest code diff (~5 lines + 1 test). The
second TUI launches with `OFFICINA_SESSION_TIER=cloud` and the router
treats it as a cloud-native session. The first TUI keeps the default
(local-first). No changes to the daily-driver env file.

### Option D — second engine slot (`--parallel 2`)
Restart the engine with two slots. Both TUIs get local service; the
router's existing spill logic handles contention. This is the
"right" long-term answer but costs VRAM + a unit restart that kills the
active turn.

**Verdict**: the real fix, but out of scope for "just run a second
session now."

## Recommendation

**Option C** is the smallest correct change that matches the owner's
ask ("the router knows to route elsewhere"):

1. Add `OFFICINA_SESSION_TIER` env read in `adaptive-router/index.ts`
   (~5 lines): `"cloud"` → force cloud tier at boot, skip local resolve;
   `"local"` (default) → current behaviour.
2. Add `OFFICINA_SESSION_PRIVACY` env read: if `"local-only"`, refuse to
   route a confidential turn to cloud (the current default `privacy !=
   safe → local` already does this, but make it explicit for the cloud
   tier).
3. One integration test: `OFFICINA_SESSION_TIER=cloud` + busy local →
   first turn goes to the Caduceus candidate, not the single slot.
4. Document the launch recipe in `docs/officina-guide.md`:
   ```bash
   OFFICINA_SESSION_TIER=cloud officina
   ```
5. No changes to `~/.vitriol/officina/env`, no unit restart, no
   fingerprint diff (the engine config is untouched).

**Invariants** (this work must preserve):
- I1: daily-driver TUI behaviour unchanged when `OFFICINA_SESSION_TIER`
  is unset (default `local`).
- I2: privacy gate still runs — confidential prompts never route to a
  cloud lane regardless of tier.
- I3: no new env key in the provenance schema (it's a per-launch launch
  override, not a persisted config); the drift detector stays read-only.
- I4: `OFFICINA_NO_OVERFLOW=1` still wins over `OFFICINA_SESSION_TIER`.

## Effort
- Code: ~10 lines in `adaptive-router/index.ts` + 1 test.
- Docs: 1 section in `docs/officina-guide.md`.
- One commit, gate before commit (npm test + selfcheck + canaries).
