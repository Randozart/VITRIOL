# Officina — The Workshop Guide

> *"The forge, the crucibles, the stills."* Officina is the interactive workshop
> where the model runs: sessions, models, gateway lanes and cloud routing,
> driven from the Rust TUI (`officina`).

## Usage

- **The prompt**: `┌──(☿ ALKA)-[model: …]-[context: …]-[drift: …]` then
  `└───> `. Telemetry blocks are toggled in `~/.vitriol/officina.toml`.
- **Model lanes** (`/model`): `officina/lapis` ("Lapis Occultus") is the
  local-first resilient default; `officina/nimbus` is the **cloud-only**
  lane — every turn routes to the best verified-FREE cloud model, the
  local engine is never selected, and the lane dies loudly (no silent
  local fallback) when no free lane fits the context.
- **Lane health** (`/nimbus`): caduceus marks unhealthy endpoints on its
  provider connections — `isActive=false` owners are excluded from lane
  selection before the first wasted turn, `backoffLevel>0` owners are
  demoted behind healthy ones, and provider-health cooldowns (429/5xx)
  rotate lanes reactively. `/nimbus` prints the lane table with gateway
  marks, cooldowns, and the next pick.
- **Autofill**: press **Tab** to cycle keyword/target completions (shown on the
  ⧉ bar under the prompt). BackTab still cycles tabs.
- **Scroll**: **PgUp**/**PgDn** scroll the output; Up/Down recall command
  history; Enter runs a command.

## Subscriptions via Caduceus

Caduceus is Officina’s name for its local gateway integration; the upstream
OmniRoute vendor remains separately installed and MIT-attributed.

Officina remains engine-first through the resilient `officina/lapis` default
(local when up, Caduceus failover when down). To use subscription providers
such as OpenCode Go, run:

    vitriol caduceus setup --opencode-key-stdin < keyfile
    vitriol caduceus status

Then open `vitriol officina` and select `/model` → `caduceus/<model>`. The
choice persists. The resilient virtual default prefers the local VITRIOL engine
and falls back to Caduceus only when it is busy or down.
`vitriol caduceus up|down|sync` manages the gateway. See `docs/OFFICINA.md`
for the passthrough contract and rollback steps.

Zero-key free providers:

    vitriol caduceus setup --free
    vitriol caduceus doctor
    vitriol caduceus service install
    vitriol caduceus use caduceus/ddgw/gpt-5.4-mini       # persist cloud default
    vitriol caduceus accounts free --template    # fill-in free API-key list
    vitriol caduceus accounts free --apply
    vitriol caduceus accounts discover --apply   # reuse OpenCode auth

Guided free-tier signup (all 123 API-key providers, one at a time):

    vitriol caduceus signup              # open pages + paste keys (hidden)
    vitriol caduceus signup --status     # progress
    vitriol caduceus signup --reset      # start over

Check what is actually configured:

    vitriol caduceus keys              # present keys, masked, with source
    vitriol caduceus keys --all        # include missing entries
    vitriol caduceus accounts status   # provider + env name + gateway link
    vitriol caduceus preflight         # local/gateway/keys/keepalive/cooling
    vitriol caduceus stats --today     # route/token usage

In the TUI: `/profile auto|fast|deep|local`, `/why`, `/free`, `/cost`,
`/keys`, `/providers`.

Session safety:
```bash
officina --new                         # force a clean session
vitriol caduceus session-doctor        # validate the newest session
vitriol caduceus recover-session       # offline only; refuses while TUI runs
```
The TUI holds `~/.vitriol/officina/tui.lock` and quarantines corrupt
sessions (`.corrupt-*`) at startup instead of resuming them.

Bulk key import (one file, one pass):

    vitriol caduceus accounts import-keys ~/keys.env   # KEY=VALUE bundle
    vitriol caduceus accounts enable --present --sync  # enable all present keys
    vitriol caduceus accounts keys --all               # verify present/missing

Bulk accounts:

    vitriol caduceus accounts init       # ~/.vitriol/officina/accounts.json
    vitriol caduceus accounts sync       # API keys via credential_env refs
    vitriol caduceus accounts status
    vitriol caduceus accounts oauth list
    vitriol caduceus accounts oauth login openference
    vitriol caduceus accounts oauth login agy
    vitriol caduceus accounts oauth login freebuff

API-key providers import in bulk. OAuth is assisted, not unattended: browser
or device consent is still required except `cursor`/`zed` system imports.
Web-cookie and local providers stay manual.

Config toggles:

    vitriol opencode                       # import an OpenCode session
    vitriol settings                       # full-screen searchable settings TUI
    vitriol caduceus config set cache auto
    vitriol caduceus config set praetor on

## Cloud cache, Praetor, egress

Cloud cache mode is automatic for non-local providers:

    vitriol caduceus cache enable --providers opencode-go,glm
    vitriol cache-report

Praetor is advisory and opt-in:

    OFFICINA_PRAETOR=1 vitriol officina

Fixed egress only (no quota-based IP rotation):

    vitriol caduceus egress set --proxy socks5://127.0.0.1:1080
    vitriol caduceus egress status
    vitriol caduceus egress test

See `docs/OFFICINA.md` for cache-safe rules, Praetor budgets, and egress policy.

## Codex-inspired QoL

- **/status** — session snapshot: model, agent mode, routing profile, ctx
  fill, cwd, session, permissions, gateway lanes, routing policy.
- **/review** — read-only review of `uncommitted` / `commit [ref]` /
  `base [branch]` / a custom focus. The review turn cannot edit (write
  tools are blocked); findings land in `.officina/review.json` and open in
  **/review-findings** (severity-sorted, j/k, detail pane).
- **Alt+E** — edit the composer draft in `$VISUAL`/`$EDITOR` (the TUI
  suspends and restores; the draft is never lost).
- **^o** — attach the clipboard image (Wayland `wl-paste` / X11 `xclip`,
  magic-byte validated, 8 MiB / 4 images); ⎙N chip in the composer title;
  **alt+o** clears. Images ride the next send as attachments.
- **Resume filter** — type in `/resume` to narrow sessions by title/path.
- **Composer predictions** — one muted "next step" line after a turn
  (failing gates → fix; dirty tree → commit). `OFFICINA_NO_PREDICTIONS=1`
  disables.
- **Sandboxed mode** — a fourth permissions mode: bash runs in a bwrap box
  (read-only filesystem, writable cwd + /tmp, no network unless
  `OFFICINA_SANDBOX_NET=1`). bwrap missing → runs unwrapped with one
  warning. ask/auto/yolo are untouched.
- **/mode is gone** — the old alias shadowed typing `/model`; agent modes
  live at `/agent` (the legacy `/mode plan` widget format still parses).

## Caduceus page

`/caduceus` opens the gateway page (full-screen, four tabs):

- **Providers** — every account plus the OAuth registry: routing on/off,
  rank, key presence, gateway bucket, free model count, signup method.
  `space` toggles routing, `+`/`-` rank, `enter` detail, `r` reload.
- **Models** — tool-capable lanes with derived free/paid flags.
  `space` cycles pin → deny → neutral, `/` filters, `h` hides the selected
  model (display only — routing is untouched) and `H` shows hidden rows.
- **Health** — gateway reachability, catalog freshness, budget, cooling,
  connection buckets (read-only).
- **Policy** — `free_only`, per-owner cap, max candidates, per-provider
  `allow paid`. `space` toggles, `+`/`-` adjusts.

Facts come from `~/.vitriol/officina/caduceus-state.json` (written by the
caduceus extension every 60 s). Policy lives in
`~/.vitriol/officina/caduceus-prefs.json`, written by the TUI and honoured
by the routers on the next candidate-pool rebuild — no restart needed.

Free-ness is derived from gateway pricing and the costless-owner list; the
page only ever toggles policy. Removing prefs (or a corrupt file) restores
the default routing exactly; the page footer says when prefs were ignored.

Adding a provider:

- `a` — quick key: an extension dialog asks for the key (visible while
  typing), writes the env file, enables the account and syncs the gateway.
- `A` — hidden-entry signup wizard: the TUI suspends,
  `vitriol caduceus signup --only <provider>` runs, then the TUI resumes.
- `o` — OAuth providers: suspend and run
  `vitriol caduceus accounts oauth login <id>`; API-key providers open
  their signup page.

Accounts remain gateway state (`vitriol caduceus accounts … --sync`); the
TUI only changes routing preferences.

## Running multiple sessions

No flags, no per-launch env — the router adapts per session, per turn
(multi-session routing, 2026-11-10; hardening 2026-11-11):

- Every Officina TUI publishes its turn decision to the routing registry
  (`~/.vitriol/officina/routing-registry.d/<pid>.json`, one shard per
  TUI, 60 s TTL) — no prompt text, only ids/tokens/tier/model/role.
  Shards are written independently, so concurrent TUIs never overwrite
  each other's entries. The legacy single
  `routing-registry.json` is no longer written.
- The arbitration lives in **resilient-default** (lapis is the default
  model); adaptive-router mirrors it for non-virtual selections.
  All sessions start local-first (Lapis Occultus). When every engine slot
  is committed, the **junior** session yields to a free cloud lane; the
  senior session (oldest, then largest context) keeps the stone. When a
  slot frees, the junior returns — its KV is reused by the engine's LCP
  slot matching.
- Switch-cap fairness (2026-11-11): peer demotions never charge the
  cache-affinity cap, returning to local resets it, and an idle local
  sheds accrued switches — a session can no longer strand itself after
  two contention events.
- Free-lane auto router (2026-11-11): every cloud lane is a verified
  free model (zero-priced or costless owner — opencode-go, dahl,
  freeinference, agentrouter, …; `OFFICINA_OVERFLOW_FREE_ONLY=1`
  default). A hard turn on an idle local promotes itself to the best
  free lane (`OFFICINA_OVERFLOW_COMPLEXITY_PROMOTE=0.65` in the env
  file; 0 disables). Paid ascensus escalation stands down while
  free-only is on — classification stays live as a status line.
- Sub-agent visibility (2026-11-11): children run without the router,
  so the parent publishes a `role: subagent` registry entry for the
  child at spawn and removes it at completion — peers see (and rank
  last) a sub-agent while it holds the local slot.
- Second TUI in the same project (2026-11-11): a kernel-held flock per
  session store fences auto-resume. A second TUI refuses to resume the
  same session file and starts a fresh session (notice printed at
  launch, `/resume` to switch later); repair/quarantine also refuse
  while another TUI owns the store. Locks auto-release on crash.
- Cache discipline (2026-11-11): the session ledger re-sites at the
  context TAIL each turn and tool-result stubbing defers until ctx
  fill ≥ `OFFICINA_CACHE_CLEAR_PCT` (92) — mid-history rewrites used to
  truncate the engine's KV prefix reuse and re-prefill the whole tail
  every turn.
- Checkpoints/snapshots are session-scoped (2026-11-11): KV autosave/
  restore gate on live peers (slot may be foreign at parallel=1) and
  markers must carry the session stem; git snapshots live under
  `refs/trismegistus/turns/<session-stem>/<n>` so concurrent sessions
  never clobber each other's rewind points (flat refs stay readable).
- Privacy still beats utilization: confidential turns never route cloud
  by default. Engine-down failover is unchanged.
- `parallel` stays 1 unless the operator wants true dual-local; the
  routing works identically at `parallel=1` (one senior local, the rest
  cloud).
