# Research: Seam Resistance — making Officina immune to layer-boundary bugs
(2026-11-20, RESEARCH mode. Companion to caduceus-router-reliability-2026-11-20.md.)

## 1. The pattern, formalised

Every recent incident was a **seam failure**: two modules, each internally
correct, disagreeing at their boundary.

| Incident | Seam | Both sides "correct" because |
|---|---|---|
| Router bypassed (049ab57 research) | env file ↔ base-url.ts P1 rule | env file documents its precedence; P1 honours explicit env |
| KV cap breach (05e5e9a/de8fbd6) | turnkeys.ts filename contract ↔ server-task.cpp disk_scan regex | each matched its own patterns |
| model_select stale header | pi extension bus ↔ RPC bridge | pi's design: bus-only emission |
| jiti ×3 | extension module instances | each instance keeps its own state |
| Vitriolum drift (CI 44719008266) | palette.json ↔ TWO TUIs' parity tests | both tested; one wasn't run |

Local survey (this session):
- 65 extensions, 173 OFFICINA_* knobs in config.ts, **29 distinct
  process.env.X names read across 24 _shared files**, ~8 config surfaces in
  ~/.vitriol/officina/ (env, agent-mode.json, caduceus-*.json ×4, compact-on-
  finish.json, accounts.json), plus ~/.pi/agent/settings.json and
  ~/.vitriol/config — **11+ places truth can live**.
- Seam guards that exist: 41 _shared test files; exactly 2 are
  cross-boundary guards (boundary.test.ts, jiti-isolation.test.ts).

## 2. External practice (web)

- **Twelve-Factor III (Config)** (https://12factor.net/config): "Store config
  in the environment" — but critically, *one* store. Backing-service handles
  and per-deploy values have exactly one resolution order, stated in one
  place. Officina violates the spirit: the same truth (provider base URL)
  resolves differently depending on which layer reads it.
- **LiteLLM** (docs/proxy/reliability): one config file declares the whole
  fallback chain; no per-layer overrides. Cost: less flexible per-surface.
  Officina needs per-surface flexibility (extensions vs TUI vs proxy), so the
  lesson is not "one file" but "one *resolver*".
- **Postel's law caution** (the principle behind "be liberal in what you
  accept"): liberal parsing at seams is exactly what let the KV scanner miss
  turn files silently. Modern practice (e.g. Zig's explicit error sets,
  Rust's typed parsing) prefers **strict inputs + loud errors over silent
  acceptance**. The disk_scan silently ignoring unknown files is Postel gone
  wrong; it should have logged unmatched *.bin files once at startup.

## 3. The four mechanisms (ranked by leverage)

### M1 — Contract Registry with drift alarms (highest leverage)
Every cross-layer contract gets a **single module** exporting both the
producer and consumer sides, plus a test that asserts they agree. The
filename contract is the canonical case:

```
// _shared/kv-contracts.ts
export const TURN_FILE = { suffix: ".bin", infix: "-turn-" };
export const ENGINE_SCANNER_ACCEPTS = [
  "cache-", "slot", "*-turn-*.bin" /* documented, mirrored in C++ comment */
];
```

Test: for each contract, parse the consumer side (the C++ source, the shell
script, the Rust file) with a cheap regex and assert the contract string
appears. This is what turn-checkpoint-budget-self-test.cpp does manually —
generalise it. **Cost: ~2 days. Catches: the entire filename-contract class.**

### M2 — Config Resolution Oven (single resolver)
One `_shared/resolve-config.ts` that owns ALL precedence (shell > env-file >
unit-detection > default) and returns a frozen object. Layers stop reading
`process.env` directly (24 files today) and read the resolved snapshot. One
debug dump: `vitriol doctor` prints the whole resolved snapshot + which rule
won at each key. The router bypass becomes impossible to reintroduce because
base-url.ts can no longer see raw env. **Cost: ~3 days, 24-file mechanical
migration. Catches: the entire precedence class.**

### M3 — Seam Tripwires (cheap, do first)
A CI job that greps for the *shapes* of known seam bugs:
- any `process.env.OFFICINA_*` read outside resolve-config.ts (warning list)
- any new filename pattern in _shared/*.ts without a matching pattern in the
  engine scanner test
- any `catch { res.destroy(); }`-style silent swallow in scripts/
- two parity-testers of the same JSON must both be listed in one place
  (so a palette.json change knows how many consumers to check — the
  vitriol-tui miss). **Cost: ~half a day. Catches: recurrences, not new
  classes.**

### M4 — One-Command Observability (`vitriol doctor`)
Every incident investigation this week started with "which layer actually
served this?" — answered by hand each time (ports, logs, session greps).
A `vitriol doctor` that prints: resolved config snapshot (M2), which lane
served the last N turns (reroute log tail), which units are running, which
files the engine scanner sees vs the extension wrote, kvtier budget state.
**Cost: ~1 day. Pays: every future incident's time-to-diagnosis.**

## 4. What NOT to do

- **Don't collapse the layers.** Six layers is the architecture's ambition,
  and each earns its keep (the TUI/extension split survived the language-
  boundary question; the proxy/gateway split survived the bypass). Fewer
  layers = fewer seams but less capability. The gotcha rate is the price of
  the design; these mechanisms make the price predictable instead of
  surprising.
- **Don't chase zero.** M1–M4 close the *known classes*. New classes will
  appear — that's discovery, not regression. The metric to watch is
  time-to-diagnosis, which M2+M4 directly shrink.

## 5. Recommended order

M3 (half day, immediate) → M2 (the structural fix) → M4 → M1 (largest, do
when a filename contract bites next).
