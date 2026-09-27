# Officina Parity Plan — 2026-09-27

**Goal**: daily-driver competitiveness vs OpenCode. Scope locked: personal tool,
parity gaps first, own roadmap (read-ahead digests, churn investigator) queued after.

**Status**: APPROVED (owner, 2026-09-27). Phase 1 in progress.

## Ground truth (researched 2026-09-27)

- pi 0.83.0 already ships `@file` fuzzy completion (`CombinedAutocompleteProvider`,
  pi-tui `dist/autocomplete.d.ts:17-54`) and the extension hook
  `ctx.ui.addAutocompleteProvider` (`types.d.ts:62`, wired at
  `dist/modes/interactive/interactive-mode.js:1700-1702`). Officina never calls it.
  P2 composer patch is layout-only (`build-patch.mjs:49-53,160`) — autocomplete untouched.
- pi has non-interactive mode: `--print, -p` (`dist/cli/args.js:109-110`),
  `--mode text|json|rpc`, `PrintModeOptions` (`dist/modes/print-mode.d.ts`).
  `tris code` argv-passes through `os.execvpe` (`cli/bin/trismegistus:125-148`);
  only blocker is the alt-screen claim in `officina.mjs:171-190`
  (`TRIS_NO_FULLSCREEN=1` skips it). Untested end-to-end until Phase 1.1.
- Generic provider type exists: `api: "openai-completions"`
  (pi `docs/custom-provider.md:225-231`). `llama-cpp-provider` is data-driven
  from `officina/models.json` + user override (`$LITTLE_CODER_MODELS_FILE` /
  `~/.config/little-coder/models.json`), registers via `pi.registerProvider`
  (`index.ts:116-121`), has `/v1/models` discovery (`index.ts:180+`).
- No MCP / LSP anywhere in pi dist or extensions (grep: zero hits).
  pi `docs/usage.md:296` states MCP intentionally excluded → extension work.
- Zen (OpenCode's gateway) = OpenAI-compatible endpoint for all free models:
  `https://opencode.ai/zen/v1/chat/completions`; catalog at
  `https://opencode.ai/zen/v1/models`; auth via `OPENCODE_API_KEY`
  (owner has key). Explicit no-lock-in policy — sanctioned third-party use.
- diagnostics-loop delivery path: `lc-diagnostics` custom tail message,
  1050-char budget, output tail 400 chars, once-then-clear
  (`diagnostics-loop/index.ts:63-77`) — reuse for LSP diagnostics.

## Phase 1 — quick wins — COMPLETE 2026-09-27

Commits: `95fee61` (1.1), `3f61103` (1.2), `8cf6410` (1.3), `a5de5ca` (1.4).
569 extension tests green; tsc clean (pre-existing jiti TS2307 in the
committed jiti-isolation test file — separate debt).

### 1.1 Headless — DONE ✅ (criterion exceeded: 4s round-trip)
Extra findings fixed beyond the plan (found by bisecting the 180s hang):
- `background-lane` launch loop was un-gated in headless → hasUI gate
  (kept the process alive forever in print mode).
- `_shared/engine.ts` poll interval now `unref()`ed (defense in depth).
- `llama-cpp-provider` model-sync retry (12×5s) spun headless — print ctx
  has no modelRegistry, so sync could never succeed; single-shot when
  `hasUI === false`.
- shipped `models.json` port drift 8888→8279 killed (user override file
  already pointed at 8279 — the drift only hit no-override hosts).
- During verification the engine appeared to "hang": actually slot 0 was
  chewing an unrelated 87.7k-token prefill at ~151 t/s (another client);
  requests queue behind it. Not a bug; noted so future 60s first-tokens
  aren't misdiagnosed.
- `tris` CLI interpreter is `~/venvs/tris/bin/python` (shebang); needs that
  venv (PyYAML), not system python3.

### 1.2 @-mentions — DONE ✅
- Verified: patched runtime retains `CombinedAutocompleteProvider` +
  `addAutocompleteProvider` (interactive-mode.officina.js:775,777,2086) —
  stock @file works under the layout fork untouched.
- New `at-mention` extension wraps the provider; adds skills/knowledge/
  protocol cards (absolute paths) + `.officina/` artifacts, capped 6.
  17 unit tests. Interactive smoke still pending an owner TUI run.

### 1.3 Zen free tier — DONE ✅ (pending owner key)
- 9 free models as `zen` provider in models.json + `/coupling zen-free`
  (live couplings.json now also carries the ascensus entry — no longer
  pre-drafted). Swap target: space-bunny-free (zero-retention).
- Live-wire verified: `--model zen/space-bunny-free` reached Zen's real
  endpoint and got 401 AuthError (key not yet stored) — full HTTP path
  proven, activation = uncomment `OPENCODE_API_KEY` in
  ~/.vitriol/officina/env.
- muse-spark contributor excluded (train-on-prompts trade); jev excluded
  (systemone API). Paid tier absent by design. 65536 conservative windows.
- auto-route untouched (cloud tier = ascensusd-only, injection-only) —
  routing can never leak turns to Zen; privacy law in OFFICINA.md.

### 1.4 /init — DONE ✅ (deviation recorded)
- Deterministic fs scan (manifests/dirs/language census/docs) → seeds
  `.officina/MEMORY.md`; confirm-before-overwrite; 5 unit tests.
- DEVIATION from plan: repo-map (tree-sitter shim) not reused — needs a
  subprocess warm-up, wrong dependency for a cold-start command. Plain
  scan is deterministic + zero-token.

## Phase 2 — MCP bridge
- `mcp-bridge` extension: vendor `@modelcontextprotocol/sdk` (MIT — license
  table allows, NOTICE entry), stdio transport first.
- Config: project `.officina/mcp.json` + global `~/.vitriol/officina/mcp.json`.
- MCP tools → `pi.registerTool` bridge (JSON-Schema→TypeBox conversion),
  namespaced `mcp__<server>__<tool>`.
- Server registry = `globalThis` singleton (`__officinaMcpState`) +
  jiti-isolation guard test (contract: `.pi/extensions/_shared/jiti-isolation.test.ts`).
- Lifecycle: spawn on session_start, health, shutdown on session_shutdown.

## Phase 3 — LSP diagnostics
- `lsp-diagnostics` extension: own minimal stdio JSON-RPC client (Phase 2
  patterns), per-project server config (`.officina/lsp.json`: clangd,
  typescript-language-server, rust-analyzer).
- `didOpen` after edit/write (hook `tool_result` like diagnostics-loop),
  `publishDiagnostics` → inject through the `lc-diagnostics` tail path
  (same budget: 1050 chars, tail 400, once-then-clear).
- Server processes: lazy start per language, kill on session_shutdown.

## Phase 4 — sidebar unblock (P11) — riskiest, isolated
- Reproduce FOCUS-TEST input-routing bug (pi 0.83.0 double
  construction/double delivery of focusable components).
- P11 anchored patch in `build-patch.mjs` fixing the routing bug;
  canaries extended. Then: scrollable sidebar, collapsed trees, persisted toggle
  (deferred items from sidebar plans).
- Fallback: upstream pi fix → pin bump (40 extensions must re-verify).

## Phase 5 — export
- `/export`: session JSON → self-contained local HTML, reuse hand-maintained
  `patched/markdown.officina.js` renderer. No cloud.

## Backlog (post-parity)
- Jev 1.13 Free (`/zen/v1/systemone`, structured yes/no/choice/score) as
  verdict engine for churn investigator + auto-route classification.
- Own roadmap 2026-09-01 items: read-ahead digests, churn investigator,
  structured-output verdicts, lane-armed indicator, glyph fallbacks.

## Process
- Commit per stage. `scripts/selfcheck.sh` gate before each commit.
- 454+ extension tests stay green; hand-maintained runtime files
  (session-selector, markdown) never regenerated; after ANY build-patch change
  run `node runtime/build-patch.mjs` and confirm `canaries ok`.
