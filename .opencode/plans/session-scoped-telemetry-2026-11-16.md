# Session-scoped telemetry: a session must see ITS OWN provider's metrics

- Date: 2026-11-16 (investigation 08:20–08:40 local)
- Mode: PLAN (read-only). No code changed; this is the plan.
- Baseline: `9422001`. Installed TUI `4a54aeba…`.

## Report

> "Not all sessions use VITRIOL, yet they all see VITRIOL's metrics, rather
> than cloud API or Nimbus metrics. Can we make sure a session listens to its
> own …?"

Confirmed, and it is a display-scoping bug, not a data-plumbing bug: every
VITRIOL-specific sidebar row renders unconditionally, while the per-session
facts that would describe a cloud/nimbus session already exist on disk.

### What renders today, and why it is wrong

| surface | code | wrong for a cloud/Nimbus session |
|---|---|---|
| `div1 "Engine"` header | `session-panel/index.ts:237-243` | a whole group that does not apply |
| `local up/down` | `session-panel/index.ts:190-232` (prio 12) | the VITRIOL engine's health, not the provider's |
| `cooling N` | same row | gateway-wide count, not this session's lane |
| `eng <n> tok/s` + "N decoded this boot" | `session-panel/index.ts:282-317` (prio 25) | only `undefined` when `!eng.up`; otherwise always |
| `loaded` (engine model/alias) | `session-panel/index.ts:318-345` (prio 26) | the engine's model, not the session's |
| `cache` (engine KV entries) | `session-panel/index.ts:268-281` (prio 21) | engine-only concept |
| `vitriol-decode` widget | `vitriol-decode/index.ts:110-120` | same rows again, `belowEditor`, in every UI session |
| poller start | `session-panel/index.ts:601-603` `if (ctx.hasUI) startEnginePolling()` | `/metrics` + `/slots` every 700 ms for a session that never touches the engine |

`_shared/engine.ts:364` `startEnginePolling()` is a globalThis singleton with no
notion of *whose* telemetry it is; `getEngineSnapshot()` is the local engine's
truth and nothing else. So a session in `~/Projects/briev-lang` on
`caduceus/opencode-go/…` shows "local up / eng 25 tok/s / 45M decoded this boot"
— the VITRIOL box's numbers, in someone else's workshop.

### The per-session truth that already exists (unused by the sidebar)

- `_shared/route-ledger.ts:167` `lastRouteEntry(sessionId)` — every routed request
  is logged with `sessionId`, `tier`, `provider`, `model`, `tokensIn/Out`,
  `cacheRead/Write`, `latencyMs`, `costEstimate`, `outcome`
  (sample: `{"provider":"caduceus","model":"dahl/MiniMaxAI/MiniMax-M2.7",
  "tokensIn":69481,"tokensOut":976,"latencyMs":44706,"outcome":"ok",…}`).
- `_shared/provider-health.ts` — `status`, `cooldownUntil`, `successCount`,
  `failureCount`, `errorClass` per `<provider>/<model>`.
- `_shared/lane-health.ts` — the `/nimbus` lane table (`getLaneHealth`,
  `LaneRow`, `formatLanes`).
- `ctx` (context fill) and `tasks` are already per-session:
  `ctx.getContextUsage()` and task-state's file stem (`sessionFileStem`,
  file `~/.officina/tasks/<session>.json`). **The task list needs no change** —
  the plan pins that with a test instead of touching it.
- Provider ids in play: `llamacpp` (local engine), `officina` (virtual
  `lapis` / `nimbus`), `caduceus`, `opencode-go`, plus pi's built-ins.

## Decisions (locked)

- **S1 Scope drives display only.** A `TelemetryScope` is derived from the
  session's provider + model at `session_start` and on `model_select`; nothing
  about routing, the model, or the agent changes.
- **S2 Engine rows are engine-scope only.** `div1`, `loaded`, `cache`,
  `engine`, and `ready`'s `local up/down` render only for scope `engine`.
- **S3 Every other scope gets ONE row describing that session's own work**, at
  priority 24 (above the engine's 25):
  - `cloud` → `api <provider>/<model> · <in>→<out> tok · €<cost> · <latency>s · <ok|error>`
    from **this session's** last route entry, plus `· cooling <n>s` or
    `· <n> fails` when provider-health says so.
  - `lanes` (`officina/nimbus`) → `nimbus <k> lanes · next <id> · cooling <n>`
    from `_shared/lane-health.ts`.
  - `none` (no provider known yet) → no telemetry rows at all.
- **S4 Silence beats a foreign number.** Never fall back to the local engine for
  a non-engine scope, even when the engine is up.
- **S5 Local parity.** A `llamacpp` session's rows stay byte-identical to today
  (same text, same order, same colours).
- **S6 Poller ownership is explicit.** session-panel starts the shared poller
  only for engine scope; it never stops one another consumer needs
  (`background-lane`, `adaptive-router`, `vitriol-decode` own their own reasons).
  Making the poller refcounted is a follow-up, listed as a non-goal here.
- **S7 The decode widget obeys the same scope** (`vitriol-decode`): engine lines
  only for engine scope, otherwise clear the widget.
- **S8 The render path stays O(1) per tick.** The last-route lookup is a cached,
  bounded tail read (session id + mtime, ~2 s TTL), never a 2-day JSONL parse on
  a 700 ms sidebar tick.

## Plan

### Stage 1 — scope (pure, testable)

New `officina/.pi/extensions/_shared/telemetry-scope.ts`:

```ts
export type TelemetryScope = "engine" | "lanes" | "cloud" | "none";
export function telemetryScope(provider?: string, modelId?: string): TelemetryScope;
```

Rules: `llamacpp`→`engine`; `officina`+`lapis`→`engine`; `officina`+`nimbus`→
`lanes`; any other non-empty provider→`cloud`; empty→`none`.
Tests: matrix over the five real ids + unknown + empty (`telemetry-scope.test.ts`).

### Stage 2 — row formatters (pure)

New `officina/.pi/extensions/session-panel/telemetry.ts`:
`formatApiRow(entry, health, now)`, `formatLaneRow(facts, now)`,
`apiRowVisible(scope)`, with the existing `sc`/`truncate` colouring injected or
returned as plain text (the panel truncates).
Tests: a cloud row from the observed ledger sample; cooling suffix; error
outcome; no entry → `undefined`; lane row from lane-health facts.

### Stage 3 — `_shared/route-ledger.ts`: bounded session lookup

`lastRouteForSession(sessionId, { cacheMs = 2000 }): RouteLedgerEntry | null`
— reads *today's* ledger only, from the **tail** (last 64 KiB), stops at the
first matching `sessionId`; memoised on (file mtime, sessionId).
`lastRouteEntry()` keeps its current behaviour for other callers.
Tests: matches the observed sample shape; torn last line tolerated; cache
invalidates on mtime change.

### Stage 4 — `session-panel` wiring

- `let scope = telemetryScope(providerName, modelId)` recomputed in
  `session_start` / `model_select`; `touch()` already forces a re-render.
- `div1`, `cache`, `loaded`, `engine` return `undefined` unless
  `scope === "engine"`; `ready` drops its `local` line in other scopes (keeps
  `cloud keys`, `profile`, `cooling`).
- New section `api` (prio 24) rendering per S3.
- `startEnginePolling()` only when `scope === "engine"`.
- `hints`/`session`/`tasks`/`scratchpad`/`files` untouched (already per-session).

### Stage 5 — `vitriol-decode`

Gate `render()` on scope: engine scope → today's lines; otherwise
`ui.setWidget("vitriol-decode", [], …)` (explicit clear) and do not start the
poller.

### Stage 6 — glossary + docs

`officina/tui/src/tui/state.rs` `SIDEBAR_GLOSSARY` gains `api` and `lanes`
entries (a Rust change → rebuild + reinstall the TUI, per AGENTS).
Update this plan with as-landed notes.

## Invariants

- **P1 local parity** — a `llamacpp` session renders exactly today's rows
  (pinned by a golden text test on the formatters + the section gates).
- **P2 no foreign telemetry** — engine/cache/loaded rows are unreachable for a
  non-engine scope; pinned by the scope matrix and the section gate test.
- **P3 own-session attribution** — the api row is built from an entry whose
  `sessionId` equals this session's; another session's request can never appear
  (pinned in `route-ledger.test.ts`).
- **P4 silence is valid** — `none` scope and unknown facts render nothing rather
  than a default.
- **P5 no new I/O on the agent path** — all new reads are sidebar-render only,
  cached, and never throw.
- **P6 tasks stay session-scoped** — unchanged code; pinned by an explicit test
  that two task files (different session stems) yield their own items.

## Gates

`npx vitest --run .pi/extensions/session-panel .pi/extensions/_shared` ·
`npx tsc --noEmit` · `cargo test` + `cargo fmt --check` (glossary edit) ·
rebuild + reinstall the TUI · live check: open a `caduceus` session and a
`llamacpp` session side by side and confirm each shows only its own row
(the engine rows must disappear in the cloud session, and the api row must name
that session's last request).

## Non-goals / follow-ups

- Refcounting the shared engine poller across consumers (`background-lane`,
  `adaptive-router`, `resilient-default` legitimately want engine state even in
  cloud sessions for local-capacity/failover decisions). Today's plan only stops
  *session-panel* from starting it for non-engine scopes.
- The `coupling` row (`◈ <name>`, engine endpoint identity) is VITRIOL-specific
  but it is the workshop anchor, not a metric; left as-is unless asked.
- pi-side queued follow-ups after an operator Esc (carried over from the
  previous plan).

---

## As landed (2026-11-16, 08:41)

Commit `36db70c` (feature) + `f0f3467` (rendered-sidebar harness). TUI rebuilt and
installed (`c82f727f…`), which also picks up the Tab fix from `b1dd259`.

### Deviations from the plan (both deliberate)

1. **Lanes come from `caduceus-state.json`**, not from `resilient-default`'s
   private picker (`loadCandidates`/`pickCloud`). That file already publishes
   `candidates[]` (99 lanes with `context_window`/`free`), `cooling`, `gateway_up`
   and `budget`, so no new writer and no duplicated selection logic: the row
   picks the first free, non-cooling lane whose context fits.
2. **The `api` row is three short lines, not one.** The sidebar is 34 columns;
   a single `api model · usage · latency · outcome · cost` line truncated the
   facts away (caught by the rendered-sidebar harness). Now: model (last path
   segment via `shortModel()`), then `69.5k→976 tok · 44.7s`, then
   `ok · free · cooling 2m`.

### Files

- `_shared/telemetry-scope.ts` — `telemetryScope(provider, model)` +
  `engineRowsVisible`; 3 tests.
- `session-panel/telemetry.ts` — `apiRowParts`, `laneRowParts`, `shortModel`,
  `fmtTokens`, `fmtCost`, `cachedJsonFile` (mtime+TTL); 12 tests.
- `_shared/route-ledger.ts` — `lastRouteForSession()` split into
  `mtimeOf`/`readTailLines`/`newestForSession` (Praetor clean); 3 tests.
- `session-panel/index.ts` — scope state (session_start + model_select), engine
  gates on `div1`/`cache`/`engine`/`loaded` and `ready`'s `local` line, new `api`
  section (prio 24), poller start gated.
- `vitriol-decode/index.ts` — widget + FIRE line gated; `model_select` handler.
- `tui/src/tui/state.rs` — glossary entries for `api` and `nimbus`.
- `session-panel/scope.test.ts` — installs the real extension, drives
  `session_start` per provider and renders the **real** section registry.

### Verification

`vitest session-panel/_shared/vitriol-decode/composer-predictions` **299** ·
`cargo test` 285 · `cargo fmt --check` · `tsc --noEmit` · Praetor 0 new
diagnostics on both commits · PTY smoke test of the installed binary.

Live check on the next launch: a `caduceus` session shows `api <model>` and no
`local up`/`eng`/`Engine` rows; a `llamacpp` session is unchanged; an
`officina/nimbus` session shows `nimbus`/`cooling`/`next`/`budget`.
