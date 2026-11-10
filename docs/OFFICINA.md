# VITRIOL Officina — the native coding workshop

**2026-08-31, owner decision:** Officina is folded into VITRIOL as the
engine's native programming environment. Source of truth:
`officina/` in THIS repo. The trismegistus repo is a historical archive
(Rule 14) — new work lands here.

## The flow

    vitriol serve          # engine up (any lane)
    cd ~/some/project      # workshop opens for THIS directory
    vitriol officina       # full-screen workshop, Vitriolum theme

`vitriol officina [args]` execs `officina/officina.mjs` (args pass
through to the scaffold CLI: `-p "one-shot"`, `-c` continue, `-r` resume).
OFFICINA_DIR overrides the location for non-standard installs.

## Subscription providers via Caduceus (2026-11-09)

Officina keeps the local VITRIOL engine as its default provider. A local
Caduceus gateway (powered by the MIT-licensed OmniRoute vendor) adds subscription providers (starting with OpenCode Go)
without weakening that coupling: an explicit Caduceus selection persists
across sessions; the resilient `officina/lapis` virtual default still prefers the local engine and fails over to Caduceus.

    vitriol caduceus setup --free --opencode-key-stdin < ~/.config/opencode-go.key
    vitriol caduceus doctor
    vitriol caduceus service install
    vitriol caduceus use caduceus/ddgw/gpt-5.4-mini   # persist cloud default
    vitriol officina        # then /model -> caduceus/<model>

What `setup` does:

- installs pinned `omniroute@3.8.53` under
  `~/.vitriol/vendor/omniroute` (PATH-independent);
- starts the headless gateway on `127.0.0.1:20128`;
- adds the OpenCode Go credential with `providers add --credential-stdin`;
- creates a local inference key for Officina;
- disables compression, semantic cache, and memory globally; prompt cache
  stays on;
- writes `OFFICINA_CADUCEUS*` to `~/.vitriol/officina/env` (mode 600).

`setup --free` additionally runs free onboarding, reuses existing OpenCode auth,
and enables provider cache overrides. `use <model>` writes the pi default
provider/model so new sessions open directly on Caduceus; `use --local
<model>` restores the local default. `doctor` is a read-only health report;
`service install` writes `~/.config/systemd/user/vitriol-caduceus.service`,
enables it, and best-effort enables lingering so the gateway survives reboot.

Strict passthrough is also enforced per request by the Officina provider,
so a global toggle drift cannot silently rewrite prompts:

    X-OmniRoute-No-Cache: true
    X-OmniRoute-Compression: off
    x-omniroute-no-memory: true

The provider catalog is fetched live from
`GET /v1/models?prefix=alias`, filtered to active Caduceus connection
providers via the local management API, mapped to pi model metadata, and
cached at `~/.vitriol/officina/caduceus-models.json`. Alias mode is
required for no-auth providers (`ddgw/`, `cfp/`, `unc/`). `vitriol caduceus
sync` refreshes the cache; pi's own `/model` refresh also re-fetches.

Operational notes:

- `vitriol caduceus up|down` controls the gateway.
- `vitriol caduceus status` reports gateway health, provider connection,
  catalog size, and whether the inference key is present (masked).
- If the gateway is down, Officina keeps cached models or falls back to
  the local engine; startup is never blocked.
- Rollback: `vitriol caduceus down`, remove the `OFFICINA_CADUCEUS*`
  lines from `~/.vitriol/officina/env`, re-enable cache/compression in the
  Caduceus dashboard. The local engine is untouched.
- Kill switch: `OFFICINA_CADUCEUS=0` prevents the provider extension from
  registering.

### Bulk accounts (API keys + OAuth assist)

Caduceus 3.8.53 ships 375 catalog providers. Accounts automation is split
by what can be automated safely:

- **API-key providers (252)** — bulk import. `vitriol caduceus accounts sync`
  builds one `providers import` payload from `accounts.json`.
- **OAuth (28 total, 8 CLI flows)** — assisted. Browser/device consent stays
  interactive; `cursor` and `zed` can import local system credentials.
- **Web-cookie (35), local (14), noauth (9)** — manual/dashboard or explicit
  opt-in; no credential scraping.

Zero-key free providers and existing local credentials:

    vitriol caduceus accounts free --json       # preview
    vitriol caduceus accounts free --apply      # add eligible free providers
    vitriol caduceus accounts free --plans      # 123 free API-key providers + signup URLs
    vitriol caduceus accounts free --template    # write them disabled to accounts.json
    vitriol caduceus accounts discover --json   # preview OpenCode auth reuse
    vitriol caduceus accounts discover --apply  # import OpenCode auth keys

`accounts free` uses Caduceus's free-onboarding API and currently adds the
working no-auth providers: DuckDuckGo AI Chat, Cloudflare AI Playground, and
AI Horde. `uncloseai` is excluded because its public beta is closed. Naga.ac
free models need a (free) account key, so they are not auto-added. Existing
`~/.local/share/opencode/auth.json` entries are reused without printing or
copying secrets: `opencode-go`, `zai`, and `zai-coding-plan` → `glm`.

DuckDuckGo and Cloudflare are browser-backed. `accounts free --apply` installs
the pinned Playwright package and Chromium if missing; `--no-browser` skips
that check. The free-onboarding API is idempotent and skips configured
providers.

Create and edit the config:

    vitriol caduceus accounts init
    $EDITOR ~/.vitriol/officina/accounts.json
    vitriol caduceus accounts sync --dry-run
    vitriol caduceus accounts sync
    vitriol caduceus accounts status

`accounts.json` references env vars, never literal secrets:

```json
{
  "accounts": [
    {
      "provider": "opencode-go",
      "name": "opencode-go",
      "credential_env": "OPENCODE_API_KEY"
    },
    {
      "provider": "openai",
      "name": "openai-main",
      "credential_env": "OPENAI_API_KEY",
      "default_model": "gpt-6-luna"
    },
    {
      "provider": "azure-openai",
      "name": "azure-prod",
      "credential_env": "AZURE_OPENAI_API_KEY",
      "provider_specific_data": {
        "resourceName": "example",
        "apiVersion": "2024-10-21"
      }
    }
  ],
  "oauth": ["claude-code", "codex", "copilot"]
}
```

`sync` is idempotent by provider + name; existing connections are skipped.
`--only ID` filters, `--strict` treats missing env vars/config errors as
failures, `--json` emits a machine-readable report with secrets omitted.
The temporary import file is mode 600 in a mode-700 directory and deleted
in `finally`.

OAuth assist:

    vitriol caduceus accounts oauth list
    vitriol caduceus accounts oauth login cursor
    vitriol caduceus accounts oauth login copilot
    vitriol caduceus accounts oauth login codex
    vitriol caduceus accounts oauth status

Native flows: `gemini`, `antigravity`, `cursor`, `zed`, `kiro`,
`claude-code`, `codex`, `copilot`. `cursor`/`zed` default to
`--import-from-system`; the rest require browser/device consent. `kiro`
also needs `--social google|github`.

Free OAuth added by the Officina wrapper:

    vitriol caduceus accounts oauth login openference
    vitriol caduceus accounts oauth login agy
    vitriol caduceus accounts oauth login qoder
    vitriol caduceus accounts oauth login freebuff
    vitriol caduceus accounts oauth login amazon-q   # alias of kiro

`openference` and `agy` are browser PKCE flows (paste the callback URL/code).
`freebuff` is a device-code flow. `qoder` is browser auth-code but stays
disabled unless the Caduceus server has `QODER_OAUTH_*` configured.
`amazon-q` delegates to Kiro's social flow. Web-cookie providers remain
dashboard/manual.

Provider-specific fields (Azure resource/version, Bedrock region/keys, Vertex
project, etc.) go verbatim in `provider_specific_data`; Caduceus validates
them at import time.

## OpenCode session import (2026-11-09)

Officina can import an OpenCode session as a native pi session:

    vitriol opencode                 # TUI picker, current project
    vitriol opencode list --json
    vitriol opencode import latest --dry-run
    vitriol opencode import <id> --no-launch
    vitriol opencode import <id> --fork

Defaults import the whole transcript; `--last N` and `--max-chars N` are
explicit trims. Tool outputs are capped at 500 chars each by default (4k with
`--full`) because OpenCode can store multi-GB tool bodies. Imported sessions
are written under `~/.pi/agent/sessions/` and opened with
`vitriol officina --session <path>` unless `--no-launch` is passed. Source DB
is read-only.

## Settings TUI (2026-11-09)

`vitriol settings` opens a searchable full-screen menu for **every key in
`~/.vitriol/config`** plus the Officina env file. Inline config comments become
the explanations; known env keys have curated descriptions.

- `↑/↓` move, type to search, `Enter` edit/cycle, `Esc` quit.
- Enum settings cycle; free-form values open a one-line editor.
- Engine config writes are atomic and backed up once as `~/.vitriol/config.bak`.
- Engine changes need `systemctl restart vitriol-server`.
- Officina env changes apply to the next session.
- Secret env keys are shown masked and are not editable in the TUI; use
  `$EDITOR ~/.vitriol/officina/env`.
- Non-interactive inspection: `vitriol settings --list` or `--json`.

## Cloud cache mode (2026-11-09)

Officina stays low-context on the local engine. When a non-local provider is
selected, `OFFICINA_CACHE_MODE=auto` switches to cache-safe behavior:

- system prompt, tool schemas, and earliest history stay byte-identical;
- dynamic context remains tail-only;
- `tool-result-clearer` only mutates the prefix near the context ceiling
  (`OFFICINA_CACHE_CLEAR_PCT`, default 92; force with
  `OFFICINA_CACHE_FORCE_CLEAR=1`);
- compaction/model switches are explicit cache epochs (expected misses).

Provider-native caching:

    vitriol caduceus cache status
    vitriol caduceus cache enable --providers opencode-go,glm
    vitriol caduceus cache disable --providers glm

`cache enable` sets `providerSpecificData.cache.supportsPromptCaching=true` on
the Caduceus connection, so Caduceus injects `prompt_cache_key` where the
upstream accepts it. Verified: `opencode-go/glm-5.2` reported
`cached_tokens=3840/4024` on the second identical request. Revert a provider
with `cache disable` if it ignores or rejects the hint.

Telemetry:

    vitriol cache-report                 # latest session
    vitriol cache-report --cwd .         # current project's sessions
    vitriol cache-report --file S.jsonl --json

The sidebar cache row shows cumulative `cacheRead`, `cacheWrite`, and an
approximate hit rate. `cache-report` computes missed tokens/cost, idle-TTL
misses, and model switches from pi session JSONL.

Env:
- `OFFICINA_CACHE_MODE=auto|local|cloud|off`
- `OFFICINA_CACHE_CLEAR_PCT=92`
- `OFFICINA_CACHE_FORCE_CLEAR=1`

Convenience toggles (written to `~/.vitriol/officina/env`):

    vitriol caduceus config get
    vitriol caduceus config set cache auto|local|cloud|off
    vitriol caduceus config set praetor on|off

## Native Praetor (advisory)

Praetor is a first-class advisory utility, off by default:

    vitriol caduceus config set praetor on
    vitriol officina

Equivalent: `OFFICINA_PRAETOR=1 vitriol officina`.

- injects `praetor instruct` once (stable block);
- after edit/write, copies the changed source file to a temp directory and runs
  `praetor validate --warn --json --target <tmpdir>`;
- appends a bounded diagnostics tail on failures (`OFFICINA_PRAETOR_BUDGET`,
  default 300 chars; timeout `OFFICINA_PRAETOR_TIMEOUT_MS`, default 60s);
- exposes `praetor_report` for `praetor report --target <cwd> --format markdown`.

Advisory only: it never blocks edits. Kill switch: `OFFICINA_NO_PRAETOR=1`.
`TRIS_DIAG_PRAETOR=1` remains a compatibility alias for the older
diagnostics-loop path. Missing `praetor` disables cleanly.

## Fixed egress (VPN / proxy)

Egress is **fixed-only by policy**. No automatic IP rotation and no
quota-triggered proxy switching — those are anti-abuse/ToS risks.

    vitriol caduceus egress status
    vitriol caduceus egress set --proxy socks5://127.0.0.1:1080
    vitriol caduceus egress set --proxy http://user:pass@host:8080 --no-proxy 127.0.0.1,localhost
    vitriol caduceus egress clear
    vitriol caduceus egress test

Config: `~/.vitriol/officina/egress.json` (mode 600). The proxy URL is applied
to the Caduceus daemon via `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY` plus
`NO_PROXY`; Caduceus's network layer consumes those variables. `set/clear`
restarts the daemon when running; `status` masks credentials; `test` checks the
egress IP through the proxy. Kill switch: `OFFICINA_EGRESS=0`.

OS-level WireGuard is documented separately; Officina does not manage root
network configuration.

## Project branding: `.officina` vs `.pi` (2026-09-02)

Two namespaces, two owners:

- **`.pi/`** — the ENGINE's dir. pi-coding-agent's upstream discovery
  convention (extensions, skills, themes, settings) plus the pinned
  runtime hooks. Never renamed — forking the vendor convention breaks
  discovery and upstream sync.
- **`.officina/`** — OUR per-project artifacts, written by officina's own
  extensions: `SCRATCHPAD.md` (hot notebook), `tasks/<session>.json`
  (task state), `approved-plan.md` (plan-mode), `background/<stem>/`
  (review cards). Reads fall back to the old `.pi/...` locations so
  pre-shim sessions keep resolving; writes always go canonical.

The TUI bridge (`officina/tui`) treats a project with either
`.pi/extensions` or `.officina/extensions` as self-curating and carries
no foreign extensions into it; bare projects get the officina core
(agent-mode, vitriol-decode, session-panel) loaded explicitly.

## What's inside (officina/)

- `officina.mjs` — entry: pins pi-coding-agent (Apache-2.0 library),
  binds our extensions + theme, claims the terminal (alt-screen +
  OSC 11 background #0d1117, restored on every exit).
- `.pi/extensions/` — 25 extensions: our harness pieces (vitriol-decode
  live engine telemetry widget, small-lane compaction on the mellum2
  lane, rewind, vitriol-checkpoint, permissions-guard, tool pipeline,
  hermes-bridge, scratchpad hot notebook) + mined load-bearing upstream (subagent, plan-mode,
  deep-research, skill/knowledge-inject, llama-cpp-provider).
- `theme/officina.json` — Vitriolum palette (BG #0d1117, PANEL #161b22,
  sovereignty gold, safety green, solvent cyan, substrate red).
- `skills/` — knowledge, protocols, tools reference cards.

## Session management

- Prior sessions: `/resume` picker, `/tree` navigator (per-project/cwd).
- `/history`: scrollable full transcript (↑↓ pgup/pgdn G q).

## Headless mode (2026-09-27)

`tris code -p "prompt"` (or `vitriol officina -p ...`) runs non-interactive:
pi's print mode with all officina extensions bound. No fullscreen claim,
no retry loops — answer on stdout, exit code carries failure.
`--mode json` streams the full event record. Background jobs, engine
telemetry polling, and model-sync retries are hasUI-gated and stay off.

## Engine KV residency tiers (2026-09-27)

Three tiers, in spill order: VRAM (live slots) -> host RAM prompt cache
(`--cache-ram 12288`, was 2048 — measured slot states run 2.5-7 GiB
bloated; the old cap rejected every mega-session by construction) ->
Optane disk mirror (`--slot-save-path`, inner commit `15686347f`:
async spill on swap, boot-time reload). Engine restart + same-context
next turn now costs seconds (certified: SIGKILL, restart, same prompt =
cached 745/755 tokens, 0.5s reply vs 14s cold; 190K-token sessions
previously re-prefilled for ~7 minutes at ~450 t/s).

Known issues (recorded, not fixed):
- Fork's seq-state serialization under `--kv-unified` bloats states
  ~60-230x vs theoretical KV size (19.1 KiB/token); spill mirrors the
  bloat symmetrically, so function is correct, footprint is fat.
- Requests with explicit `id_slot` bypass the prompt cache entirely
  (upstream dispatch semantics: update_cache only set on LCP-loss/LRU).
- `llamacpp:kv_cache_spill_total` wiring unproven (reads 0 after spills
  in earlier process; unverified in a spilling process).
- Mega-session spills are multi-GB writes; boot loads them back
  (~40s for 9.8 GB on top of model load).

## @-mentions (2026-09-27)

The composer's `@` fuzzy completion (stock pi `@file`, fd-backed) is
extended by `at-mention` with officina-owned targets: skill/knowledge/
protocol cards (`officina/skills/**`, inserted as absolute paths so they
resolve from any project) and `.officina/` artifacts (SCRATCHPAD.md,
MEMORY.md, approved-plan.md, tasks/*). File hits stay stock; officina
items are appended, capped at 6.
- `/panel`: right sidebar — modified files, tokens, model, session id.
- cwd in the terminal title + widget line at all times.

## Design law

docs/officina-archive/SCAFFOLD-SOVEREIGNTY-2026-08-31.md (local copy;
originals frozen in the trismegistus archive repo), First-Party Mandate: upstreams (little-coder, hermes, OpenCode,
Crush) are mining sources only; pi-coding-agent is a pinned library, not
a shipped frontend; the HTTP API is the only engine surface.

## Standing TODO (2026-08-31, owner dispositions)

1. ~~small-lane A/B~~ **DROPPED** by owner — no measurement required; the
   small lane is policy, not a claim.
2. `vitriol lanes` GPU lane arbiter — APPROVED, build next.
3. Per-slot n_kv in /slots — APPROVED, engine-side, cert-gated; makes the
   decode widget's busy truth exact.
4. little-coder fallback removal — APPROVED after a parity ledger entry.
5. Gateway replacement — APPROVED, next major build: memory, chat, and
   injection safety move off hermes-agent into Officina (hermes-bridge,
   tris chat, memory-extractor, caveman-rules, injection-guard are the
   seeds). Last non-VITRIOL runtime in the stack.
6. 27B cert run — APPROVED; owner has validated the harness on 27B and
   loves it ("we have something amazing").
7. Retention/GC + checkpoint consolidation — APPROVED.

## Engine vs Officina boundary (2026-08-31, owner question)

Rule: **VITRIOL owns what must be true of the MACHINE; Officina owns what
must be true of the WORK.** Currently in VITRIOL that should move or be
built Officina-side:

| Item | Where it lives now | Should be |
|------|--------------------|-----------|
| skills/ knowledge, protocols, tool cards | officina (correct) | Officina — agent procedure, not engine |
| unified harness config (safety.permissions, pipeline switches, budgets) | trismegistus archive config.yaml | Officina — policy is harness law |
| session ledger + events (ledger.jsonl, events.jsonl) | ~/.local/state/trismegistus | Officina (rename state dir ~/.vitriol/officina) |
| tok/task accounting, success rates, agent benchmarks (benchmark-profiles, deep_research_runs) | officina | Officina — measures the WORK; engine `bench` (llama-bench) stays |
| small-lane compaction POLICY (when/what to compact, mellum2 choice) | officina ext | Officina — engine only hosts a second server if asked |
| turn-key / rewind mapping, snapshot ext | officina | Officina — engine serves checkpoint bytes; turn semantics are harness (fixes F7 dual-writer) |
| permissions/tool gating, read/write guards, plan mode | officina | Officina — governance over the WORK |
| pymander reference-mind store | scripts/vitriol | candidate for Officina/gateway (memory domain) during #5 |
| autosave boot-restore | vitriol-autosave.service | split: slot serialization stays ENGINE; when-to-save/restore policy moves Officina |

Stay engine-side (never move): llama.cpp fork, DMA executor, KV/cache
managers, server + /slots /metrics /health /props + checkpoint endpoints,
cert suite, profiles, fingerprint emission, GPU lane scheduling, bench.

## Couplings (2026-08-31)

The default coupling is **Lapis Occultus – VITRIOL**: whatever settings
VITRIOL is running with, regardless of which model that is — the endpoint
is the coupling, the model behind it incidental. Alternative providers are
hot-swappable mid-session (pi.setModel preserves the conversation):
`/coupling` lists, `/coupling <id>` swaps, `/coupling lapis-occultus`
returns to the stone. Couplings come from
~/.vitriol/officina/couplings.json (shape: officina/couplings.example.json).
This is the groundwork for a built-in ascensus tool. The session panel's
cordoned top frame shows the current coupling name.

### Live couplings (2026-09-27, parity phase 1.3)

- `zen-free` — OpenCode Zen free tier (`https://opencode.ai/zen/v1`,
  `OPENCODE_API_KEY` in ~/.vitriol/officina/env). Swap target:
  space-bunny-free (zero-retention). Nine free models also register as the
  `zen` provider via models.json (pick natively with `/models zen/…`).
  **Privacy law**: free-tier terms let Big Pickle / MiMo / Ling train on
  prompts; NVIDIA Nemotron trial endpoints forbid confidential data;
  Space Bunny + LongCat declare zero retention; muse-spark contributor
  models are EXCLUDED entirely (explicit training trade). No confidential
  material on this tier, ever. Declared context windows are conservative
  65536 (catalog publishes none); paid models (glm-5.3-flash,
  qwen3.8-flash, deepseek-v4-flash, …) are deliberately absent — add to
  your user-override models file if wanted.
- `ascensus` — euro-capped cloud escalation via ascensusd (now a live
  coupling entry, no longer pre-drafted).

auto-route is untouched by zen: its cloud tier is ascensusd-only
(injection, no model switch), so sensitive/confidential turns can never
reach Zen through routing — Zen use is always an explicit /coupling or
/models choice by the owner.
PROVENANCE: catalog/endpoint reference only (opencode Zen docs); zero
code copied.

## Layout notes (2026-08-31)

- Composer pinned to the screen floor: the wrapper pre-pushes the render
  origin by rows-8, so a fresh session opens with the typing field at the
  bottom; content scrolls naturally once it outgrows the reserve
  (Crush/OpenCode behavior).
- Panel colors: the cordoned top frame uses Vitriolum accents on values
  only — gold coupling, solvent folder, safety-green token flow, violet
  files, muted labels/keys — never full-screen color wash.

## Bugfix log

- 2026-08-31 (1): editor untypeable at startup — the sidebar overlay
  captured keyboard focus. Fix: OverlayOptions.nonCapturing (the purpose-
  built flag), replacing the unfocus-on-show hack.
- 2026-08-31 (1b): typing STILL dead after nonCapturing — root cause
  found by PTY reproduction: the v2 (widget-based) panel never actually
  shipped; a failed file write left v1's focus-stealing overlay in the
  tree while the commit message claimed v2. v2 re-landed and verified by
  driving the real TUI in a pseudo-terminal — keystrokes now echo in the
  editor. LESSON: TUI bugs get PTY-reproduction tests before "fixed" is
  claimed. The ui.custom overlay approach is abandoned for the sidebar. Panel v2 renders via
  setWidget (structurally incapable of capturing input); /history stays
  an intentional modal. DOCKED-RIGHT sidebar (Crush/OpenCode layout)
  requires a horizontal layout primitive pi-tui lacks — next build item:
  vendor + fork the interactive-mode layout (editor pinned bottom is
  already pi's natural layout; the right column is the fork).
- 2026-08-31 (2): `vitriol officina` opened with a first prompt
  "officina" — the launcher's case never shifted, so the subcommand word
  stayed in "$@" and pi read it as a positional prompt. Fix: shift first.

### Session UI fix + agent modes (2026-08-31, evening)

- TYPING BUG ROOT CAUSE (PTY-proven): ANY persistent extension overlay —
  even with nonCapturing — breaks keyboard routing in pi-coding-agent
  0.83.0. Bisection: panel ext off = typing works; on = dead, with keys
  reaching neither the overlay nor the editor. Upstream finding; the
  session panel therefore stays on the widget system until the layout
  fork. (Earlier "fixed" claims had shipped stale v1 code through a
  failed file write — both mistakes now guarded by the PTY test.)
- agent-mode ext: /mode plan|build. Plan injects a research-first
  directive (cache-safe tail message) and blocks write/edit on non-.md
  targets at the tool_call gate with a reason; bash mutations are NOT
  parsed (documented limit — the directive covers them). Build removes
  the gate and injects a one-shot "writes allowed again" hint.
  Kill switch OFFICINA_AGENT_MODE=0.
- Panel v3b: cordoned, colored (gold coupling / solvent folder /
  safety tokens / violet files), widget-rendered above the editor,
  composer pinned at the screen floor.

### Watermark (2026-08-31)

officina-header ext: the VITRIOL braille logo (assets/braille-logo-80c.txt,
80 cols, 40 rows) renders watermark-style as the session header on an
untouched session — tinted #1c2634, a faint blue lift off the #0d1117
background. First user input starts the chat and the logo scrolls away
naturally (PTY-verified: 3200 braille cells at startup, gone from the
viewport once the conversation takes over). Missing asset = silently no
watermark. Kill switch OFFICINA_WATERMARK=0.

### TAB mode toggle (2026-08-31)

TAB toggles Plan/Build (default Build). PTY-verified both directions:
TAB shows the plan indicator + research directive, TAB again clears it.
Note: this reclaims the editor's autocomplete binding (tui.input.tab) —
deliberate tradeoff, the mode toggle is the more valuable binding here.

### Mode-switch inference bug (2026-08-31, owner-reported)

TAB/mode switch fired a model turn with the directive as its prompt —
the switch used pi.sendUserMessage, which STARTS A TURN. Fixed: mode
directives now ride along invisibly on the user's own next turn via the
before_agent_start event (plan directive rides every plan turn; the
build hint rides exactly once and is consumed). No turn is ever fired by
switching modes. Startup note: a one-time "keybinding conflict" banner
for TAB is informational — the extension binding wins.

### Conflict banner silenced (2026-08-31)

officina.mjs now idempotently ensures ~/.pi/agent/keybindings.json has
"tui.input.tab": [] (TAB unbound from autocomplete — claimed by the
agent-mode toggle). Merge-preserving: existing keybindings survive and an
owner-managed tui.input.tab entry is never touched. PTY-verified: startup
banner gone, typing intact.

### Fold-in completion (2026-08-31)

VITRIOL is now self-contained. The tris CLI moved to officina/cli
(~/.local/bin/tris retargeted; 41 tests green), the unified config
detached from the old repo into ~/.config/trismegistus/config.yaml (real
file), and the canonical design docs live in docs/officina-archive/.
The trismegistus repo (now private) is pure history — nothing on this
machine requires it anymore.

### SS1 + SS5 landed (2026-08-31)

- SS1: little-coder fallback removed from the workshop launcher; validate
  parity retargeted to officina models.json (shipped default merged with
  ~/.config/officina/models.json; legacy little-coder path is read-only
  fallback during transition); hermes custom_providers dropped from port
  parity. User model override migrated to ~/.config/officina/.
- SS5: officina/scripts/selfcheck.sh — external-path grep gate, vitest,
  typecheck, cli pytest, config + keybindings checks, opt-in --live
  engine smoke. Run before every release tag.
- Remaining: SS2 gateway fold-in, SS3 repo-map, SS4 state rename.

### SS2a — gateway fold-in phase one (2026-08-31)

Officina owns memory. New `memory` ext (provenance: successor to
hermes-bridge + hermes memory-extractor concept, trismegistus hermes-plugins
@ 237e424): store ~/.vitriol/officina/memory/{MEMORY,USER}.md, tools
memory_read / memory_write / memory_search (own-session JSONL scan),
per-turn hidden injection of facts via before_agent_start (cache-safe).
LIVE-VERIFIED: the 27B wrote a fact to MEMORY.md through memory_write.
hermes-bridge ext retired (its only consumer was the memory contract);
`tris chat` retired — `vitriol officina` is the conversation surface.
injection-guard ported to TS (provenance header cites the hermes guards.py
origin): log mode default, block opt-in via TRIS_GUARD_MODE, screening
browser/webfetch ingested text. Caveman-rules + memory-extractor ports
recorded as SS2b (dark features, no runtime dependency today). Gates:
typecheck, 407 tests, praetor PASS, live memory write + one-shot OK.

### SS2a-b — memory goes project-scoped (2026-08-31, owner direction)

Memory is programming memory: it belongs to the project. Two stores:
project cwd/.officina/MEMORY.md (default target of memory_write;
versionable, reviewable in diffs) and global
~/.vitriol/officina/memory/USER.md (owner facts that travel across
projects). Injection merges both, labeled. memory_search covers both
stores plus own-session files. Live-verified: project write landed in
/tmp/proj-x/.officina/MEMORY.md.

### SS2b + SS3 + SS4 landed (2026-08-31)

- SS2b caveman: deterministic prose compressor ported to TS
  (provenance: hermes compress.py @ 237e424, measured −65% upstream).
  DARK by default, armed with TRIS_CAVEMAN=1; applies to compression-
  allowed tool results only (dispatch reports, memory_search); code spans
  byte-preserved, never inflates.
- SS2b memory-extractor: user-utterance fact candidates (same four rules
  + confidences) queue to ~/.vitriol/officina/memory/curator-queue.jsonl
  for human sign-off; auto-append to project MEMORY.md only with
  TRIS_MEMORY_AUTO=1 and confidence >= 0.85. Poisoning discipline kept.
- SS3 repo-map: OFF unless a real checkout exists (OFFICINA_REPO_MAP_DIR
  or legacy var); no external clone assumed; tests updated to the new
  contract.
- SS4 state: consolidated at ~/.vitriol/officina/state; legacy
  ~/.local/state/trismegistus migrates (moves) on first CLI use.
- SELFCHECK: PASS (grep, vitest 408, typecheck, cli pytest, config,
  keybindings).

### Provenance registry + enforcement (2026-08-31)

docs/PROVENANCE.md: the complete citation index — libraries (pi pinned
Apache-2.0, typebox, dev tooling), every extension's origin (little-coder
ports @ 1a6ee8b with divergence notes; hermes plugin ports @ 237e424 with
identical rulesets; owner-authored originals), design assets (braille
bars, Vitriolum palette from our own engine TUI), retired dependencies
record. selfcheck.sh section 3b now FAILS the tree if any registered file
loses its provenance header — citations are enforced, not decorative.

### Layout fork step 2 — vendoring mechanism live (2026-08-31)

runtime/hooks.mjs: a module loader hook serves our patched
interactive-mode.officina.js to the pinned pi runtime while anchoring all
of its relative imports at the original package location — the whole
import graph stays intact with zero rewriting. pkgDist + docked flag
arrive via register() initialize data (hooks run on a separate thread).
runtime/build-patch.mjs regenerates the patched copy from the pristine
reference with asserted anchors (P1 seam applied; P2/P3 markers reserved).
Re-basing on a pi bump = bump pin, re-run build-patch, fix anchors.
Live: DOCKED-OK one-shot, 415 tests, PTY typing intact with docked active.

### Dev-process rule (2026-08-31, earned the hard way)

VITRIOL/officina is the ONLY canonical tree. The trismegistus archive is
read-only history — NEVER sync archive → canonical. A reversed rsync
briefly reverted committed files to stale versions; caught by comparing
working tree to HEAD and restored from HEAD (nothing authored was lost).
Going forward, code flows one direction only: edits land in VITRIOL.

### Layout fork steps 3-5 — docked layout is the default (2026-08-31, night)

P3: `ui.setSidebar(lines)` on the extension ui surface; session-panel
renders into the docked column when the patched mode is active (capability
probe — no env coupling) and keeps the widget path for classic.
P1: OfficinaSplit (JS port of runtime/columns.ts, injected by
build-patch.mjs; OSC/APC zero-width-aware because pi's CURSOR_MARKER is an
APC sequence) puts chat/pending/status/widgets/editor in a main column
with the sidebar docked right (34 cols, auto-hide < 100); header and
footer stay full width. P2: filler rows above the editor block pin the
composer natively; the launcher newline-reserve is classic-only.

CRITICAL FIX (PTY gate): module hooks do NOT cross process boundaries —
the fullscreen launcher spawns pi as a child, so the hook registered in
officina.mjs never applied and the child silently ran stock
interactive-mode. Fix: `runtime/register-hooks.mjs` re-registers the hook
in the child via `NODE_OPTIONS --import` (OFFICINA_PKG_DIST env).

PARITY (step 5, engine-side metrics deltas, same fingerprint Qwen3.8-27B
ts=22,14): identical task set of 3 — docked 161 tok_in / 16 tok_out
(5.3 tok/task), classic 161 / 16 (5.3 tok/task) — bit-identical; both
ledger records appended. PTY: docked frame at col 87 with chat text on
the same screen rows, composer at floor, watermark intact, typing echoes;
classic rollback verified. `OFFICINA_LAYOUT=classic` remains the rollback.

### Theme unification — Vitriolum single source (2026-08-31, night)

`_shared/vitriolum.ts` is now the single palette source for all
extensions, mirroring `vitriol-tui/src/theme.rs` (canonical) and
`theme/officina.json`. Every extension re-declared its own ANSI constants
before; now they import `fg`/`fgSeq`/helpers. Drift fixes: the widget
accent "honey" (#e15a1f, stale branding reference) is retargeted to the
theme's antidote #ff5f1f — subagent tracker, plan-mode, phase-model,
deep-research all shift one notch brighter orange; tracker ✓/✗ SGR 32/31
become safety/substrate truecolor. vitriol-decode ramps + muted and the
officina-header watermark tint now derive from the same module.
Sidebar panel tone (owner pickup, same night): the docked column renders
on the theme's panel color #161b22 (theme.rs PANEL) instead of the bare
#0d1117 substrate — the sidebar reads as a distinct surface, matching the
VS Code Officina-dark panel treatment. Applied in OfficinaSplit
(build-patch.mjs) as a per-row bg fill + reset; the gap column stays
substrate. Fit fixes (second PTY pass, pyte screen-render proof): the
sidebar slot is 42 cells so the panel's 40-cell box never clips, and the
panel now ANSI-aware-wraps its content rows to the box interior (frame
columns always align) with narrow-mode compaction: coupling model suffix
dropped, session id dropped, short key hints.

Enforcement: `_shared/vitriolum.test.ts` (4 tests) parses theme.rs
Color::Rgb triplets and officina.json vars and fails on any drift;
selfcheck section 2b greps the 9 core hexes across json + extension
palette. No palette value was replaced — the theme is the same; only the
off-palette duplicate is gone.

### Sidebar content upgrade — engine-truth cockpit (2026-08-31, night)

Mined from Crush (`sidebar.go`: context gauge, files+diffcounts,
focus-when-scrollable) and OpenCode (`sidebar.tsx`: 42-col panel,
`backgroundPanel`, context block with % + tokens). Adopted: context row
(braille capacity gauge + % + exact filled/window tokens from
`ctx.getContextUsage()` — honest `--` right after compaction), engine row
(slots gauge + tok/s or `idle` + decoded-this-boot), files with real
`+adds −dels` parsed from `EditToolDetails.patch`. Not adopted: LSP/MCP/
skills sections, cost, collapsible trees (need a scrollable sidebar).
Deferred: live plan/todo rows (needs a shared phase-state module),
session title. New: `_shared/engine.ts` — ONE shared engine poll
loop (subscribe/snapshot) now feeds both vitriol-decode's widget and the
sidebar; vitriol-decode refactored onto it, parsers still in decode.ts.

### FOCUS-TEST experiment — interactive sidebar (2026-08-31, night)

Question: can an in-layout (non-overlay) sidebar component safely take
keyboard focus in the patched interactive-mode? Rig (env-gated,
`OFFICINA_SIDEBAR_FOCUS_TEST=1`): a `FocusableSidebar` wrapper in
OfficinaSplit with `handleInput`, focused at constructor+6s, editor
refocused at +12s; PTY/pyte verdicts per phase.

Results: routing to the wrapper mechanically WORKS (`setFocus` by
reference delivers keys; wrapper echoed typed text; editor recovered on
refocus). BUT keystrokes were simultaneously echoed in the main column
AND captured by the sidebar across phases, with focus appearing to flip
against the timer schedule — evidence of MULTIPLE InteractiveMode/
TUI instances (or re-entrant input delivery) in one session: the same
keystroke reaching several focused components, and two sets of rig
timers firing. One run also showed duplicate sidebar boxes rendered.

VERDICT: an interactive sidebar is BLOCKED, not by the overlay bug, but
by an instance/lifecycle layer problem (double construction and/or double
input delivery) that must be root-caused in pi 0.83.0 before any
focusable component can coexist with the editor. Roadmap: collapsed
trees, scrollable sidebar, and persisted toggle stay deferred until that
is fixed upstream or patched in the fork. Production mode unaffected
(rig is env-gated); typing, panel, and suites all verified after the
experiment.
