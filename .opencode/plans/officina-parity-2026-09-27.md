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

## Phase 1 — quick wins

### 1.1 Headless verify ✅ criterion: `-p` round-trip answers from live engine
- Smoke: `tris code -p "2+2?"` (and `vitriol officina -p ...`).
- `officina.mjs`: when argv contains `-p`/`--print`, force `TRIS_NO_FULLSCREEN`
  behavior (skip alt-screen claim) — cosmetic for print mode, zero TUI risk.
- Document one-liner usage in `docs/OFFICINA.md`.

### 1.2 @-mentions
- Verify stock `@file` completion survives the layout fork (P1 OfficinaSplit
  replaces the stock layout; editor + autocomplete provider are editor-level,
  expected to survive). Manual smoke + note in OFFICINA.md.
- Officina autocomplete provider extension: trigger `@` + `/`; suggests files
  (fd-backed, already stock), skills (`officina/skills/**`), couplings,
  `.officina/` artifacts (SCRATCHPAD, tasks, approved-plan).
- Tests: suggestion assembly pure-function tests (jiti-isolation guard if
  cross-extension state needed — prefer none).

### 1.3 Zen free tier (owner: key held; free-only default)
- `officina/models.json`: add `zen` provider —
  `api: "openai-completions"`, `baseUrl: "https://opencode.ai/zen/v1"`,
  `apiKey: "OPENCODE_API_KEY"`, models:
  big-pickle, space-bunny-free, longcat-2.5-preview-free, mimo-v2.5-free,
  mimo-v2.6-flash-free, ling-3.0-flash-fin-free, nemotron-3-ultra-free,
  nemotron-3.5-lightning-free.
  EXCLUDED by default: muse-spark-1.3-contributor-free (trains on prompts),
  jev-* (systemone endpoint, not OpenAI-compatible), all paid tier.
  Cheap paid tier (GLM 5.3 Flash, Qwen3.8 Flash, DeepSeek V4 Flash) as
  commented opt-in block.
- `~/.vitriol/officina/couplings.json`: "Zen Free" coupling (cloud tier).
- `auto-route`: zen-free models tagged cloud-tier — sensitive/complex
  classification applies; private turns NEVER routed. Same discipline as ascensus.
- Key goes in `~/.vitriol/officina/env` (owner action, not committed).
- Rot: free tier is "limited time", deprecations regular — reuse
  llama-cpp-provider `/v1/models` discovery (`GET /zen/v1/models`) so stale
  entries surface at registration.
- PROVENANCE: inspiration line only (catalog/endpoint reference; zero code copied).
- Caveat honored: Big Pickle / MiMo / Ling / Nemotron free periods may train on
  data; NVIDIA trial terms forbid confidential data. auto-route tagging is the
  mitigation; also record in coupling description.

### 1.4 /init bootstrap
- Command `/init`: repo-map scan → seed project `.officina/MEMORY.md`
  (structure, build/test commands, conventions pointers). Reuses repo-map +
  memory extension stores. No-op if MEMORY.md exists (confirm before overwrite).

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
