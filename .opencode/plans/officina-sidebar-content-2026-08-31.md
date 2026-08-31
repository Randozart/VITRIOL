# Officina sidebar content upgrade — 2026-08-31

Mining sources: Crush (`/tmp/crush-src/internal/ui/model/sidebar.go`) and
OpenCode (`/tmp/opencode-src/packages/tui/src/routes/session/sidebar.tsx` +
`feature-plugins/sidebar/*`). First-Party Mandate: patterns only, no code.

## Goal

The docked sidebar should answer "where is this session?" — context fill,
engine throughput, and change footprint — using telemetry we already emit.
The Vitriolum visual language (braille ramps) does the formatting.

## Content decisions (mined → adapted)

| Content | Crush/OpenCode | Ours |
|---|---|---|
| Context gauge | model/context %, tokens, $ cost | **braille capacity-ramp gauge** + % + filled/window tokens (exact counts per AGENTS window≠depth law; no cost — local lanes) |
| Throughput | — (neither has it) | **slots gauge + tok/s + decoded-this-boot** (ENGINE TRUTH; we're unique here) |
| Files | changed files + `+adds −dels` | keep presence list, **add real diff counts** from `EditToolDetails.patch` |
| Todo/plan | live plan list | **deferred** — needs a cross-extension phase-state module (phase-model owns state) |
| Session title | editable title | **deferred** — no title source in pi session yet |
| LSP/MCP/skills | both show these | **not adopted** — infra diagnostics, low signal |
| Cost | OpenCode $ spent | **not adopted** — local engine; ascensus can consume ledger directly |

## Steps — ALL DONE 2026-08-31 (typecheck + 412 vitest + selfcheck PASS; PTY/pyte screen render shows ctx + eng rows in the docked sidebar with borders aligned)

1. `_shared/engine.ts` — lift vitriol-decode's poll loop into a shared
   singleton (subscribe/snapshot: up, slot gauge data, t/s delta, decoded
   total). vitriol-decode refactors onto it; `decode.ts` pure parsers stay
   put (decode.test.ts keeps passing).
2. session-panel v4 sidebar content:
   - context row: `renderGauge(RAMPS.capacity, pct, 8)` + `NN% · filled`,
     from `ctx.session.getContextUsage()`, updated on `message_end`;
   - throughput row: activity/mercury gauge + tok/s (or `idle`) + slots
     `n/m` + decoded total, from `_shared/engine.ts` (2 s poll, sidebar
     re-render on change);
   - files rows: `name +a −d` (safety/substrate), counts parsed from
     `details.patch` (+/− lines excluding +++/---), touch-count fallback;
   - narrow-mode layout rules stay (wrap to box interior, 42-col slot).
3. Verification: typecheck, vitest (decode.test.ts + panel tests), PTY
   screen render (pyte) with engine up — gauge rows visible in sidebar,
   borders aligned, typing intact; engine-down honest fallback.
4. Docs: OFFICINA.md record; PROVENANCE.md row for `_shared/engine.ts`.

## Risks / notes

- `getContextUsage()` tokens may be null right after compaction — render
  `ctx --` honestly, never invent numbers.
- Diff counts only for edit (details.patch); write shows `+lines` when the
  result content is countable, else touch marker.
- Two sidebar renders per poll tick max; setSidebar is requestRender-cheap.
