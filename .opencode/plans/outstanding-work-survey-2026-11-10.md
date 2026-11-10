# Outstanding work survey — 2026-11-10

## A. Uncommitted: the 2026-11-10 wave (top priority)

Working tree on `main` carries **58 modified/renamed files, +6510/-777** —
the entire 2026-11-10 workday is uncommitted since `9ca0f46` (11-09):

| Area | What it is |
|---|---|
| `omniroute` → `caduceus` renames (RM) | extension dir, scripts, CLI lib — part of the COMPLETE cutover (`.opencode/plans/caduceus-cutover-2026-11-10.md`) |
| `resilient-default`, `adaptive-router`, `route-ledger`, `provider-health`, profiles | the Convenience harness P0/P1 (`officina-convenience-2026-11-10.md`, "P0/P1 core implemented") |
| `officina/tui/*` (~4000+ lines) | Rust TUI features: markdown.rs, layout.rs, state.rs, rpc/bridge.rs, main.rs |
| `officina.mjs`, `build-patch.mjs`, `runtime/patched/*` | JS frontend patches for the same features (deprecated path, but keep in sync) |
| `AGENTS.md`, `NOTICE`, `docs/OFFICINA.md`, `docs/officina-guide.md` | doc updates (Caduceus naming, lapis default) |
| `scripts/vitriol` (+60) | `vitriol caduceus` CLI surface |

Tests were green at 16:12 per the plan doc (702 JS, 99 Rust, selfcheck PASS),
but **nothing has landed** — the repo's own workflow rule is "commit after
every stage". This is the single most outstanding item.

## B. Documented remaining work (newest first)

1. **Convenience harness remainders** (`officina-convenience-2026-11-10.md`):
   shared proxy for opencode/subagents (the `vitriol-router` follow-up, also
   flagged in `overflow-routing-2026-11-10.md`), model-picker badges, keyring.
2. **Great-harness pass partials** (`officina-great-harness-2026-11-10.md`):
   Phase 5: `/tree`, `/fork` list. Phase 6: live subagent orchestration test.
   Phases 4/7/8: snapshot/rewind armed but diff-review, MCP, memory UI
   polish remain.
3. **A/B: cache8-mtp2 vs mtp3** — recipe written, only "in a safe window"
   (engine restart kills active turns).
4. **Fiat Lux triad** (`fiat-lux-triad-2026-11-09.md`) — status "approved,
   implementing"; verify what shipped vs the `/coupling fiat-lux` +
   `vitriol fiat` screen scope.
5. **Standing TODO (2026-08-31, `docs/OFFICINA.md`)**: #2 `vitriol lanes`
   GPU arbiter, #3 per-slot n_kv, #4 little-coder fallback removal,
   #5 gateway replacement (big one), #6 27B cert run, #7 retention/GC.
   Several may have been overtaken by the 11-10 gateway/Caduceus work —
   worth a disposition pass.

## C. Non-urgent / external

- `vitriol-daemon/src/main.rs` — 2× "TODO: Integrate llama.cpp" (daemon
  skeleton, low priority).
- VENDOR TODOs (stb, miniaudio, xxhash, nlohmann) — leave alone.

## Plan (proposed, for approval)

**Step 1 — Land the 11-10 wave (one session, gated):**
1. `bash scripts/vitriol selfcheck` (or equivalent) + `npm test` in
   officina + `cargo test` in `officina/tui` — re-confirm green.
2. Run `node officina/runtime/build-patch.mjs` and confirm `canaries ok`
   (Vendor Patch Rule — the patched/* files are in the diff).
3. Split into logical commits (Caduceus rename; resilient-default +
   overflow/failover; route ledger + health + stats; TUI features; docs).
4. Fingerprint diff of any `~/.vitriol/config` keys touched (`cram=8192`
   change is config-side, not repo-side — no repo commit needed, but note it).

**Step 2 — Disposition pass (docs only):**
Re-read the standing TODO list against the 11-10 reality (gateway
replacement item #5 largely realized by Caduceus; mark done/obsoleted with
dates), and check Fiat Lux triad completion status in its plan doc.

**Step 3 — Next real work (pick with owner):**
- A/B mtp2 vs mtp3 (needs safe engine-restart window)
- `/tree` + `/fork` (small, Phase 5 finish)
- live subagent orchestration test (Phase 6 unblock)
- shared `vitriol-router` proxy for opencode/subagents
