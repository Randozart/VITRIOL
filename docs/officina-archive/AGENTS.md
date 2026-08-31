# Trismegistus — Agent Guidelines

**2026-08-28:** Initial operating manual. Derived from the house style
(briev-lang operating contract, VITRIOL measurement discipline, ontic sieve
rigour) and adapted to Trismegistus' division of labour between three
sovereign layers under one hard context budget. Reference material lives in
`PLAN.md` (Round 1) and `REPORT-02-EXPANSION.md` (Rounds 2-3, final execution
order).

## Operating Contract

You are building a harness whose product is **context efficiency under a hard
~54K token budget on constrained local hardware** (Qwen3.8-27B Q3_K_M on
RTX 3060 + GTX 1070 Ti). The model is the weakest component — every token
spent must earn its place. Zero tolerance: "probably fine" context bloat is a
critical failure. Every edge case in a layer you touch is solved completely
NOW — never deferred, never "out of scope," never "pre-existing."

Every decision passes three questions:

1. **Does this stretch the context budget or spend it?** A feature that
   doesn't fit the budget gets redesigned — never "a bigger window," never a
   weaker technique. The budget is the specification, not a constraint to
   negotiate.
2. **Does this respect layer sovereignty?** The scaffold (first-party as of
   2026-08-31 — see First-Party Mandate; was little-coder) owns
   what enters context; the engine (VITRIOL) owns KV-cache efficiency; the
   gateway (Hermes) owns memory, skills, and delivery. One layer doing
   another's job is the original sin this project exists to avoid (the shim
   truncating scaffold injections). The HTTP API is the ONLY engine surface
   for the agent layer.
3. **Is this measured?** No technique lands without baseline AND after
   numbers on the real hardware, same model fingerprint, controlled A/B.
   Unmeasured optimization is superstition.

Patches are unacceptable. There is no "go fast and break things."

## First-Party Mandate (2026-08-31, owner directive)

**The product is OUR harness, tightly coupled to VITRIOL — not a skin over
someone else's.** Progress to date leaned too hard on driving other
projects' frontends. That phase is over. Explicitly:

1. **little-coder, hermes-agent, OpenCode, and Crush are MINING SOURCES,
   not the destination.** Mine everything useful for a custom coding
   environment (code licensed for the purpose: Apache-2.0, MIT, FSL —
   provenance headers mandatory); drive them only for comparison runs.
   The first-party scaffold plan lives in
   `docs/SCAFFOLD-SOVEREIGNTY-2026-08-31.md` (pi-coding-agent is a pinned
   Apache-2.0 LIBRARY, never a frontend we ship).
2. **Tight VITRIOL coupling is a feature, not an integration detail.**
   Where a generic agent shows a spinner, we show ENGINE TRUTH. Standing
   requirement (owner, 2026-08-31): the tris cockpit shows live decode
   progress — tokens streamed / decode t/s / slot state from VITRIOL — in
   the main window, so the owner never needs to open the VITRIOL TUI to
   see how far along the local agent is. Any scaffold/cockpit change that
   could carry engine telemetry and doesn't is a regression.
3. **New work lands first-party.** Extensions, plugins, and loops are
   built in this repo (`scaffold/`, `hermes-plugins/`→eventually first-
   party gateway) per the sovereignty plan. Contributing an upstream patch
   is allowed; making an upstream project the runtime we depend on daily
   is not.
4. **Rule 2 layer sovereignty is unchanged** — the scaffold is OURS now;
   its ownership of context entry does not move.

## Golden Rules

1. **THE BUDGET IS THE CONTRACT**: ~54K usable tokens is the specification.
   Never weaken it to fit a feature — see `REPORT-02` §R2.8 for the current
   allocation table; a change to the table requires a dated report amendment.
2. **LAYER SOVEREIGNTY**: scaffold owns context entry, engine owns KV
   efficiency, gateway owns memory and delivery. Violations are architectural
   bugs, not tradeoffs. The shim stays flag-off for Trismegistus sessions
   (REPORT-02 §2.1); re-enabling it for a Trismegistus session is a contract
   violation.
3. **MEASURE BEFORE YOU BUILD**: every performance or token-efficiency fix
   requires a baseline table (t/s, tokens/task, task success rate) captured
   BEFORE changes and the after-table in the same report. Controlled A/B on
   the same machine, same model, same fingerprint. A refuted hypothesis
   blocks the fix. Never excuse a regression as "noise" without the A/B.
4. **FINGERPRINT PROVENANCE**: every benchmark, report, or log excerpt embeds
   the `VITRIOL-FINGERPRINT:` line and full argv (inherited VITRIOL
   discipline). Silent flag drift is a review blocker. If the fingerprint
   changed since the last report, say so explicitly.
5. **WINDOW ≠ DEPTH** (inherited verbatim): KV is allocated for the whole
   window at load. Context claims must state FILLED token counts —
   shallow-bench numbers do not certify filled-context operation.
6. **CERT GATES CONFIG**: no model or engine configuration activates in
   `~/.config/trismegistus/` without passing the VITRIOL certification suite.
   `cert_required: true` is the default and stays on; advisory mode exists
   only for R3.2 studies (REPORT-02 §8).
7. **CACHE IS SACRED**: never break the prompt/KV cache mid-conversation.
   All hooks and injections are cache-safe: system prefix frozen, additions
   appended as tail messages only (little-coder pattern). A hook that
   mutates mid-conversation history is a critical failure.
8. **CLEAR → COMPACT → COMPRESS**: the context pipeline runs in that fixed
   order (REPORT-02 §R2.10). Each stage has an independent config kill
   switch. Running stages out of order, or stacking reductions without
   monitoring the total reduction rate, is a bug.
9. **UPSTREAM STAYS UPSTREAM**: little-coder changes live ONLY in
   `.pi/extensions/` + config; Hermes changes ONLY in plugins; VITRIOL
   changes ONLY in the VITRIOL repo (vitriol branch). Core files are never
   patched in place — this keeps all three updateable. If an upstream change
   is genuinely required, fork consciously and record the divergence in a
   dated report entry.
10. **ALWAYS FINISH**: no `TODO`, stubs, or deferred edge cases in committed
    code. Every extension is wired end-to-end: config → hook → test → docs.
11. **TESTS OR IT DOESN'T EXIST**: vitest for little-coder extensions, pytest
    for Hermes plugins, plus the integration smoke test (Commands §) after
    any change that touches the request path. A technique without a test
    that can fail does not exist.
12. **NEVER DISCARD UNCOMMITTED WORK**: `git checkout --`, `git restore`, and
    `git checkout .` DESTROY work permanently — never use them. Targeted
    `git add` only. `git reset HEAD` is safe (unstaging only).
13. **FULL PROVENANCE TRACKING**: every rationale comment carries when, why,
    what it targets, and how to undo it. `// TEMP: YYYY-MM-DD:` flags
    temporary solutions with a path to permanence. Config comments carry
    when/why — configs are documentation.
14. **RECORDS ARE HISTORICAL**: VERDICTS.md, EXPERIMENT_LOG.md, dated
    reports, and `.opencode/plans/` are never retroactively edited. New
    dated entries only; amendments to live docs cite the date.
15. **KILL SWITCHES MANDATORY**: every context manipulation (tool-result
    clearing, compression, ReWOO dispatch, context relay, memory extraction)
    has an independent kill switch in the unified config, with the default
    per REPORT-02. A stage that cannot be turned off individually must not
    ship.
16. **LOCAL-FIRST, SECRETS MASKED**: no cloud calls except euro-capped
    ascensus escalation (VITRIOL policy). No telemetry. Secrets never in
    logs, reports, or fingerprints. Ingested content (PDFs, web) passes the
    injection guard before entering context (REPORT-02 §4.5).

## Protocols

### Server Lifecycle Protocol (codified 2026-08-28 session; supervisor amendment 2026-08-29)

0. **THE SUPERVISOR OWNS THE PORT ON CACHYOS**: `vitriol-server.service`
   (user unit, enabled, `Restart=always`, `RestartSec=5`) plus
   `vitriol-autosave.service` (`PartOf=`) run the engine. nohup-style
   killall gets resurrected 5s later — silently. `scripts/dev-server.sh`
   is unit-aware: start/stop go through systemctl when the unit exists;
   death is VERIFIED (pgrep + port probe) before reporting success.
1. Kill stale servers FIRST: `killall -9 llama-server` — stale servers
   silently answer on the port with old flags.
2. Load the profile explicitly: `vitriol config load <profile>` — read the
   diff output; if the profile is stale (wrong model path), fix it before
   proceeding.
3. Launch via the SUPERVISOR when the unit exists (`systemctl --user start
   vitriol-server.service`); only fall back to DETACHED nohup
   (`nohup vitriol serve > <log> 2>&1 &`) where no unit is installed — a
   foreground serve in a shell with a timeout gets SIGKILLed mid-load and
   takes the model down with it (2026-08-28 lesson). The launcher script
   `scripts/dev-server.sh` implements this protocol; prefer it.
4. Poll `/health` until `{"status":"ok"}` before ANY request.
5. Smoke test the full chain, not just the endpoint: direct curl
   (`TRISMEGISTUS-OK`) AND little-coder one-shot
   (`LITTLE-CODER-VITRIOL-LINK-OK`). The link layer is a separate failure
   surface from the engine.
6. Record the fingerprint line from serve output in any session report.

### Extension Development Protocol

1. Scaffold first: implement as a little-coder extension
   (`.pi/extensions/<name>/`), test standalone against the live server with
   `npm test` + a one-shot `-p` run. No Hermes wiring until standalone
   passes.
2. Then wire: Hermes plugin calls the scaffold; the scaffold never calls
   Hermes directly except through the `hermes-bridge` extension.
3. Every extension declares its kill switch and its token budget in the
   unified config (REPORT-02 §7, step 23).
4. Typecheck (`npm run typecheck`) before commit; `tsc` errors are commit
   blockers.

### Benchmark Protocol (R3 Gate)

1. R3 work (REPORT-02 §5) starts ONLY after execution steps 1-25 are done
   AND baselines exist: t/s, tokens/task, task success rate on the certified
   config.
2. Every benchmark embeds the fingerprint and full argv; use the sweep
   controller (`libvitriol/sweep_controller.py`) where applicable — ad-hoc
   timing produces false hangs and imprecise numbers.
3. Speculative decoding re-tests and model swaps run the VITRIOL cert suite
   first; verdicts go to VERDICTS-style dated entries, never inline edits.
4. MTP has zero benefit on this hardware (2026-05-25 sweep + 2026-08-24
   recheck) — do not re-test without a new hypothesis.

### Layer Interface Protocol

- Engine surface: OpenAI-compatible `/v1` on 127.0.0.1:8279, plus
  `/slots`, `/metrics`, `/health`. Checkpoint/restore endpoints (REPORT-02
  §2.2) are the only additional surface; nothing else is added without a
  dated report.
- Dispatch blocks on engine state: sub-coder spawn checks `/slots` KV fill
  first (REPORT-02 §2.3). The context monitor VERIFIES, it never guesses.
- Config is single-source: `~/.config/trismegistus/config.yaml` generates or
  validates per-component configs. Duplicate config files drift; drift is a
  bug.

## Architecture Pillars

- **Three sovereign layers, one contract.** Scaffold little-coder
  (Apache-2.0, Node), engine VITRIOL (Apache-2.0, C/CUDA), gateway Hermes
  (MIT, Python). All Apache-2.0-compatible since the 2026-08-28 license
  change.
- **The engine is the scheduling authority.** Slot state, KV fill, and
  certifications come from VITRIOL; the harness config refuses uncertified
  combinations.
- **The context pipeline has one order.** Clear (evict consumed tool
  results) → compact (summarize old turns) → compress (LLMLingua-2 /
  caveman-rules) — each stage independent, kill-switched, and monitored for
  total reduction rate.
- **Everything is measured.** The harness records tok/task, t/s, cache hit
  rate, and compression savings; savings claims without numbers are
  rejected in review.
- **The name is the map.** VITRIOL (the reagent), alka-* (alkahest),
  Hermes (hermeticism), Trismegistus (the binder). New components keep the
  alchemical register.

## Working Rules

- **Flat control flow** — max 2 nesting levels. Guard clauses, early
  returns; deeper logic in named helpers. `else if` chains deeper than one
  level are forbidden.
- **Continuous commits** — commit after each logical step when tests pass
  (do not ask). Targeted `git add`; never amend; never
  `git checkout --`/`git restore`.
- **Per-commit checklist**: typecheck/tests green (`npm run typecheck`,
  `npm test`, `pytest` for Hermes); integration smoke if request path
  touched; Praetor on changed files (`praetor validate --warn --target
  <dir>` — **`--target` takes a DIRECTORY, never a file**; a file target
  silently passes without analyzing anything. For a single file, copy it to
  a temp dir and target that dir); config comments updated when/why; docs
  updated in the SAME commit as structural changes.
- **Regression guard**: inspect every extension hook on touch (silent
  regressions come from removed hooks); verify token counts, not just test
  passes; never delete rationale comments — rewrite them.
- **System-level changes**: trace the full request path (client → scaffold →
  engine → back); verify claims in source (file:line), not memory; state the
  hypothesis AND its verification test, then RUN it.
- **Interpretation of numbers**: never blame "noise" without a controlled
  A/B (old vs new, full task set, same machine, same fingerprint). Document
  before corrective action.

## Commands

```bash
# Server lifecycle (implements the Server Lifecycle Protocol)
trismegistus/scripts/dev-server.sh start   [profile]  # kill, load, detached serve, health poll
trismegistus/scripts/dev-server.sh smoke   [model-id] # curl + little-coder end-to-end test
trismegistus/scripts/dev-server.sh stop                # graceful + killall fallback
trismegistus/scripts/dev-server.sh status              # health + fingerprint from log

# Direct little-coder one-shot (smoke / standalone extension test)
cd ~/Projects/little-coder
LLAMACPP_API_KEY=none ./bin/little-coder.mjs --provider llamacpp \
  --model "Qwen3.8-27B-Q3_K_M.gguf" -p "<prompt>"

# little-coder quality gates
cd ~/Projects/little-coder && npm run typecheck && npm test

# Hermes plugin gates
cd ~/Projects/hermes-agent && python -m pytest plugins/ -q

# Trismegistus front door (Round 4)
tris up|down|smoke|code|chat|go|budget|watch|validate|status
tris-watch --dump                      # cockpit snapshot, headless

# Trismegistus quality gates (unified config, step 23 surface)
trismegistus validate [--json]         # contract + drift gate; exit 1 on FAIL
~/venvs/tris/bin/python -m pytest ~/Desktop/Projects/trismegistus/tools/tests -q

# Hermes user-plugin gates (canonical trismegistus/hermes-plugins/, symlinked)
cd ~/Desktop/Projects/hermes-agent && venv/bin/python -m pytest -c ~/.hermes/plugins/pytest.ini \
  ~/.hermes/plugins/{vitriol-bridge,model-providers/vitriol,injection-guard,caveman-rules,memory-extractor}/tests -q

# Praetor (changed files) — DIRECTORY target, never a file
praetor validate --warn --target <changed-dir>
```

- **Manual server control** (when not using the script):
  `vitriol config load <profile>` then
  `nohup vitriol serve > /tmp/opencode/vitriol-serve.log 2>&1 &` then poll
  `curl -s http://127.0.0.1:8279/health`.
- **Stale model entries are bugs**: `~/.config/little-coder/models.json` and
  `~/.vitriol/profiles/little-coder/` must name the loaded model — check
  both after any model change (2026-08-28: both were stale after the
  Qwen3.6→Qwen3.8 swap).

## Reference Index

| Resource | Location |
|----------|----------|
| **Round 1 plan (architecture, budget)** | `PLAN.md` |
| **Rounds 2-3 + final 25-step order** | `REPORT-02-EXPANSION.md` |
| **Live progress ledger + driver-swap commands** | `POST-MIGRATION-PLAN.md` |
| **Unified config (canonical; symlinked to ~/.config)** | `config/config.yaml` — validate via `trismegistus validate` |
| **Cockpit/experience design (T1-T4)** | `docs/TRIS-EXPERIENCE.md` |
| **Daily-driver gap + DSL/vision/synthesis decisions** | `docs/DAILY-DRIVER-GAP.md` |
| **Rules/cache policy (residency knobs, diet, verify-first)** | `docs/RULES-CACHE-POLICY.md` |
| **Architectural audit 2026-08-30 (F1-F16 work queue)** | `docs/AUDIT-2026-08-30.md` — dispositions recorded there |
| **Ratified harness vision (why we build this)** | `docs/THESIS-2026-08-31.md` |
| **Crush mining plan (Tier B insert M1-M9)** | `docs/CRUSH-MINING-PLAN-2026-08-31.md` |
| **Scaffold sovereignty (replace little-coder; alkahest)** | `docs/SCAFFOLD-SOVEREIGNTY-2026-08-31.md` |
| **Roadmap 2026-08-30 (pillar pre-registration + Tier A/B/C order)** | `ROADMAP-2026-08-30.md` — dogfood-day rubric |
| **Hermes user-plugins (canonical; symlinked to ~/.hermes/plugins)** | `hermes-plugins/{vitriol-provider→model-providers/vitriol,vitriol-bridge,injection-guard,caveman-rules,memory-extractor}` |
| **Engine measurement discipline** | `~/Projects/VITRIOL/AGENTS.md` — REQUIRED reading before benchmark work |
| **Engine deep docs** | `~/Projects/VITRIOL/docs/ARCHITECTURE.md`, `docs/OPERATIONS.md`, `docs/VERDICTS.md` |
| **Scaffold** | `~/Projects/little-coder/` — README + `.pi/extensions/` |
| **Gateway** | `~/Projects/hermes-agent/` — plugin system docs |
| **Repo map** | https://github.com/noambinabout-boop/repo-map (step 5) |
| **Certified config** | `vitriol config load qwen38-master` (96,836 tok @ 11.32 t/s) / `qwen38-mtp-131k` (49K default) — **2026-08-31: both DEV-cert-pending** after the dual-GPU rebuild (build-ku-cu12, sm_61;86, CUDA 12.9 toolkit); live boot reports n_ctx=81920, vision e2e 12.0 t/s, fingerprint in VITRIOL/EXPERIMENT_LOG.md. Cert suite run re-activates these numbers (Rule 6). |
| **Token budget table** | `REPORT-02-EXPANSION.md` §R2.8 |
| **Execution order** | `REPORT-02-EXPANSION.md` §7 (25 steps) |

## For OpenCode

1. Read this file, `PLAN.md`, and `REPORT-02-EXPANSION.md` for full context.
2. The budget is the contract — never weaken it; redesign instead.
3. Layer sovereignty is architectural law; the HTTP API is the only engine
   surface.
4. Measure before you build; fingerprint every claim.
5. Upstream stays upstream — extensions and plugins only.
6. Run the Server Lifecycle Protocol before any integration work; use
   `scripts/dev-server.sh`.
7. Typecheck + tests + integration smoke before committing.
8. Praetor `--target` takes a directory, never a file.
9. Historical records are never retro-edited; new dated entries only.
10. Every context stage has a kill switch; defaults per REPORT-02.
