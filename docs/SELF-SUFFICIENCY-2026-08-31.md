# Self-Sufficiency Plan — VITRIOL + Officina standalone — 2026-08-31

**Goal:** clone VITRIOL on a bare machine, `npm ci`, `vitriol serve`,
`vitriol officina` — and the full experience works with **zero runtime
dependencies on any other local repo or tool**. VITRIOL is the engine AND
the workshop; nothing else is required, everything else is optional.

## Dependency inventory (audited 2026-08-31)

| Dependency | Kind | Status | Verdict |
|---|---|---|---|
| pi-coding-agent 0.83.0 | npm library (Apache-2.0) | pinned in officina/package.json | **KEEP** — a library dependency, not a runtime project. Lockfile pins it; upgrades are deliberate, tested bumps |
| node ≥ 22 + jiti | runtime | required by pi | KEEP — documented requirement |
| hermes-agent | runtime (gateway) | `tris chat`, hermes-bridge memory reads, memory-extractor, caveman-rules, injection-guard live there | **REPLACE** (SS2) — the last non-VITRIOL runtime |
| little-coder | runtime (fallback scaffold) | tris code fallback path; validate parity checks its models.json | **REMOVE** (SS1) |
| trismegistus repo | history | frozen archive | **DETACH** (SS1) — done except cosmetic names |
| repo-map MCP server | external clone (~/Projects/repo-map) | repo-map ext + shim | **VENDOR or OPTIONALIZE** (SS3) |
| Crush / OpenCode | none | mining references in docs only | DONE |

## Phases

### SS1 — Cut the little-coder + trismegistus cords (small)

- Delete the little-coder fallback from the workshop launcher path;
  `tris`/`vitriol officina` require only VITRIOL.
- Retarget `tris validate` parity checks: officina/models.json + VITRIOL
  profiles + detached unified config (drop little-coder models.json and
  hermes custom_providers from the parity set).
- Fresh-clone gate (see §Gate) passes.

### SS2 — Gateway fold-in: Officina owns memory + chat (the big one)

Today the conversation gateway is hermes-agent: the base prompt, memory
(MEMORY.md + its state.db, read via hermes-bridge), injection-guard,
caveman-rules compression, memory-extractor, and `tris chat` all ride it.
Fold the *function* into Officina, officina-side per the boundary rule:

1. **Memory**: officina-memory ext — project + global memory as plain
   markdown (`~/.vitriol/officina/memory/`), injected via the existing
   cache-safe tail-message machinery (skill/inject pattern). No SQLite
   unless the ledger proves a need; agent-writable, curator-queued.
2. **Chat**: Officina IS the chat — `vitriol officina` covers it; `tris
   chat` (hermes -z) is retired once memory parity is proven.
3. **Injection safety**: port injection-guard checks (ingested content
   screening) into the officina injection pipeline.
4. **caveman-rules / memory-extractor**: port as Officina-side processors
   on the same events they hook today (Hermes plugins are the reference).
5. **hermes-bridge** retires when memory parity is proven; a one-shot
   migration command imports existing Hermes MEMORY.md.
6. **pymander**: decide — fold the reference-mind store behind the same
   officina memory interface, or keep as an engine-adjacent tool. (Open.)

Gate: a full day of work runs with hermes-agent not running; grep gate
shows no hermes paths outside docs.

### SS3 — Optional tooling made honestly optional

- repo-map: vendor the server into `officina/tools/repo-map/` (it is a
  static Python tool) or make the ext degrade to plain grep/glob with a
  startup note. Pick one; remove the silent external-path dependency.
- Audit every remaining absolute path under /home or ~/Projects in the
  tree: `grep -rn "Desktop/Projects" officina/ scripts/ --exclude node_modules`
  must return only docs and deliberate env defaults.

### SS4 — State + config consolidation (cosmetic but load-bearing for clarity)

- State dir rename: `~/.local/state/trismegistus` → `~/.vitriol/officina/`
  with a one-shot migration (move files, keep old dir as symlink for one
  release). Git refs `refs/trismegistus/turns/*` keep their name (breaking
  existing snapshots for a label is not worth it) — recorded decision.
- Unified harness config detaches fully into `~/.vitriol/officina/config.yaml`;
  `tris validate` learns the new path and warns on the old one.

### SS5 — Self-sufficiency gate (repeatable)

A script (`officina/scripts/selfcheck.sh`) that asserts:
1. `grep -rn "Projects/little-coder\|Projects/hermes-agent\|Projects/trismegistus" officina/ scripts/ --exclude-dir node_modules`
   returns only docs/archive hits;
2. `npm ci && npm test && npm run typecheck` in officina;
3. pytest in officina/cli;
4. `vitriol serve` + one-shot `-p` smoke with the engine up;
5. PTY check: startup renders (watermark + panel) and keystrokes echo.

Run it before every release tag. Docs updated in the same commit (house
rule), fingerprint recorded for any perf-affecting change.

## What self-sufficiency does NOT mean

- Not a monorepo grab: engine layers stay engine layers (boundary table
  in this doc's parent section).
- Not pin-fear: pi-coding-agent and node are dependencies by design; the
  requirement is that *no other local checkout* is needed, and every
  upgrade is a conscious, tested bump.
- Not a rewrite of pi: we ride its release train and fork only on breakage
  (Rule 9), with the divergence recorded.
