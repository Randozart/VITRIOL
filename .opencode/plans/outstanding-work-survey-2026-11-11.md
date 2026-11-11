# Outstanding work survey — 2026-11-11 17:30

Supersedes `outstanding-work-survey-2026-11-10.md` (that "58 files uncommitted"
item has since landed).

## A. Uncommitted in the working tree (only outstanding repo-state item)

`main` is clean apart from **3 modified files, +38/-38**:

| File | Change |
|---|---|
| `officina/tui/src/tui/layout.rs` | Top-bar divider **moved from under the header to above the footer** — the 1px breathing gap now carries the BORDER_DIM rule, cordonning the bottom status band (slots/engine/ctx) from the transcript. `render_header_divider` removed. |
| `officina/.pi/extensions/caduceus/catalog.ts` | `freeinference` added to `FREE_OWNER_RANK` (rank 2, after dahl) + `COSTLESS_OWNERS` set so manual FreeInference models without a price block still qualify as free lanes. |
| `officina/.pi/extensions/caduceus/catalog.test.ts` | +2 lines for the above. |

This is a small, coherent unit (TUI divider relocation + free-lane
catalog fix) and should be one commit, gated by `cargo test` +
`npm test` + `node runtime/build-patch.mjs` canaries (build-patch not in
diff, so canary risk is low but cheap to confirm).

## B. Completed since the 11-10 survey

- **11-10 wave landed**: caduceus rename, resilient-default/lapis,
  adaptive-router, route ledger, TUI features — now in HEAD.
- **Ctx bar + divider** (`ctx-bar-divider-2026-11-11.md`): implemented —
  `get_session_stats` data source + divider; **binary rebuilt + reinstalled
  2026-11-11 15:26** (both `target/release/officina` and
  `~/.local/bin/officina`), so the AGENTS.md "updated binary" gate is met.
  The 3-file diff above is the follow-up relocation of the divider.
- Note: the ctx-bar divider originally landed as commit `189149c`
  (under the header); the pending diff moves it above the footer —
  i.e. a design correction on top of the shipped feature.

## C. Documented remaining work (newest first)

1. **Multi-session routing P1–P3** (`multi-session-routing-2026-11-10.md`):
   P1 shared routing registry + jiti test; P3 `decideTier` peer arm +
   seniority rank (pure policy, tests dominate); P2 slot pinning
   **DEFERRED 2026-11-11** (not implementable from our side — provider
   can't address slots; LCP may make it moot). I1–I6 invariants pinned
   by `overflow-peer.test.ts` + `peer-policy.test.ts`. ~2 focused
   sessions, 3 commits.
2. **Open TUI bug — composer wrap** (owner-reported 2026-11-11, tracked
   in the multi-session plan): composer box height accounts for overflow
   but text does not wrap; long unbroken tokens spill past the right edge.
   Fix lives in `officina/tui/src/tui/layout.rs` composer path:
   wrap-then-measure at (cols − prompt-glyph width), unit test with a
   200-char token in a 60-col box.
3. **Convenience harness remainders** (`officina-convenience-2026-11-10.md`):
   shared `vitriol-router` proxy for opencode/subagents, model-picker
   badges, keyring.
4. **Great-harness partials** (`officina-great-harness-2026-11-10.md`):
   Phase 5 `/tree` + `/fork` list; Phase 6 live subagent orchestration
   test; Phases 4/7/8 diff-review, MCP, memory UI polish.
5. **A/B: cache8-mtp2 vs mtp3** — recipe written; needs a safe
   engine-restart window.
6. **Standing TODO disposition** (`docs/OFFICINA.md`, 2026-08-31): #2
   `vitriol lanes` GPU arbiter, #3 per-slot n_kv, #4 little-coder
   fallback removal, #5 gateway replacement (largely realized by
   Caduceus — needs a dated disposition), #6 27B cert run, #7
   retention/GC.

## D. Proposed plan (in order)

1. **Commit the 3-file diff** (divider relocation + FreeInference
   lanes): `cargo test` + `npm test` + build-patch canaries, then one
   commit; the TUI half needs a rebuild + `cp` to `~/.local/bin/officina`
   before it counts as done (AGENTS.md gate).
2. **Composer-wrap fix** (small, same layout.rs surface, unit test
   specified) — owner-reported, visible every session.
3. **Multi-session routing P1 + P3** (the named next feature; P2 stays
   deferred).
4. **Disposition pass** on the 2026-08-31 standing TODO (docs-only).
5. Deferred until a safe window / operator pick: mtp2/mtp3 A/B,
   `/tree` + `/fork`, subagent orchestration live test, `vitriol-router`
   proxy.
