# Caduceus Panel — provider/model preference + onboarding UI

- Date: 2026-11-12 12:20
- Status: in progress (stages 1–4)
- Supersedes: the read-only `/caduceus` modal (QoL batch stage 2, commit `a7f9e19`)

## Context

The gateway surface grew three independent, invisible knobs:

1. **Enablement** — `~/.vitriol/officina/accounts.json` (`enabled`), gateway
   `isActive`, `provider-health.json` cooling.
2. **Free-ness** — derived in `officina/.pi/extensions/caduceus/catalog.ts`
   from catalog pricing plus `COSTLESS_OWNERS`; ranked by the hardcoded
   `FREE_OWNER_RANK` and `familyRank`.
3. **Routing policy** — `OFFICINA_OVERFLOW_PER_OWNER`,
   `OFFICINA_OVERFLOW_MAX_CANDIDATES`, `OFFICINA_OVERFLOW_FREE_ONLY`,
   `OFFICINA_OVERFLOW_CLOUD_FALLBACK` in `~/.vitriol/officina/env`, plus the
   `auto|fast|deep|local` profile file.

Operators cannot see, let alone change, any of this without editing files or
re-running CLI wizards. `/caduceus` renders a read-only snapshot (I4 of the
QoL batch). This plan adds a full-screen tabbed page that shows the facts and
owns a routing-policy file honoured by every router.

## Architecture

Two files, one writer each:

```
~/.vitriol/officina/caduceus-state.json   v2  extension writes — FACTS
~/.vitriol/officina/caduceus-prefs.json   v1  TUI writes       — POLICY
```

- **Facts** (existing 60 s writer, `caduceus/state.ts`): v2 adds
  `providers[]` (id, name, website, category, has_free, free_note, auth_hint,
  credential_env, key_present, signup_status, gateway bucket, free model
  count) and `candidates[]` (id, owner, context_window, max_tokens, free).
  Sources: accounts.json + env + provider-health.json + gateway provider
  catalog (`<vendor cli> providers available --json`, 10-min cache) +
  `caduceus-oauth-lib.mjs` `OAUTH_PROVIDERS` + `signup-progress.json`.
- **Policy** (Rust TUI, atomic tmp+rename): schema below. Read by the
  extensions; never written by them.
- **Enforcement**: `_shared/caduceus-prefs.ts` (`readPrefs`, `applyPrefs`)
  wired into `resilient-default`, `adaptive-router`, `auto-route` after the
  profile sort, plus a widened catalog pool (`toolCandidates`) so
  `allow_paid` can actually admit paid models.

### Policy schema

```json
{
  "version": 1,
  "free_only": true,
  "per_owner": 8,
  "max_candidates": 48,
  "providers": {
    "opencode-go": { "routing": "on", "rank": 0, "allow_paid": false }
  },
  "models": { "pinned": ["opencode-go/deepseek-v4.1-flash"], "denied": [] }
}
```

Semantics:

- `routing: "off"` — owner excluded from every candidate pool.
- `rank` — explicit ranks sort before unranked owners (ascending); unranked
  owners keep their pool order. Overrides `FREE_OWNER_RANK` ordering only,
  never enablement.
- `allow_paid: true` — that owner's paid tool-capable models may enter the
  pool when `free_only` is on. Off means free candidates only, as today.
- `free_only: false` — all tool-capable candidates allowed.
- `pinned` — first in listed order; `denied` — removed.
- `per_owner` / `max_candidates` — override the env defaults.
- Invalid file / version mismatch / unreadable → treated as absent (today's
  behaviour) and surfaced as `prefs: ignored` in the page footer.

### Free-ness is a fact, not a setting

The UI never offers "mark this free". The badge comes from catalog pricing +
`COSTLESS_OWNERS` + the owner allow-list. Users control policy only.

## UI — full-screen tabbed panel

`/caduceus` opens a full-bleed panel inside the existing modal phase (no new
navigation layer). Four tabs, tab bar on top, footer key hints, stale badge
when `caduceus-state.json` is older than 180 s.

```
┌ Caduceus ── [Providers] Models  Health  Policy ─────────────── 12:04 ┐
│ ▸ opencode-go     on   rank 0    key ✓  active   18 free   api-key  │
│   dahl            on   rank 1    key ✓  active    2 free   api-key  │
│   cloudflare-play off  rank —    key —  down     20 free   api-key  │
├─────────────────────────────────────────────────────────────────────┤
│ a quick key · A wizard · o oauth · Space toggle · +/- rank · / filter│
└─────────────────────────────────────────────────────────────────────┘
```

### Providers tab

All API-key providers from the gateway catalog plus the static OAuth list.
Columns: routing state, rank, key present, gateway bucket
(active/backoff/down), free model count, signup method.

- `Space` — routing on/off (policy only; `accounts.json` untouched).
- `+` / `-` — rank up/down.
- `a` — quick key: extension command `/caduceus-add <provider>` →
  `ctx.ui.input` → write env (atomic) → enable account → `accounts sync` →
  refresh state. Dialog title warns the key is visible while typing.
- `A` — suspend TUI, run `vitriol caduceus signup --only <provider>`
  (browser + hidden key entry), resume, reload.
- `o` — OAuth providers: suspend TUI, `vitriol caduceus accounts oauth
  login <id>`; API-key rows: open `website`.
- `Enter` — detail pane: env var, free note, auth hint, signup progress.

### Models tab

Tool-capable candidates with owner, ctx, free badge.

- `Space` — cycle pin → deny → neutral.
- `/` — filter (substring on id).
- `Enter` — detail (max tokens, owner, free source).

### Health tab

Gateway connections split active/backoff/hard-down, cooling model count,
catalog count + freshness, budget row (daily/monthly, cap, paid allowed).
Read-only.

### Policy tab

`free_only` toggle, `per_owner` (1–32), `max_candidates` (1–256), per-provider
`allow_paid` toggles, prefs validation state ("applied" / "ignored: <reason>").

### Keymap

`Tab`/`BackTab` or `1`–`4` tabs · `j/k`/arrows move · `g/G` first/last ·
`PgUp/PgDn` page · `Space` toggle · `+/-` rank/caps · `/` filter ·
`r` reload facts+policy · `Esc/q` close. Extension dialogs keep priority
(`handle_dialog_key` runs first).

## Onboarding flows

1. **Quick key** (inline, no TUI suspend): `/caduceus-add <provider>` in the
   caduceus extension. `ctx.ui.input` for the key, atomic env write, account
   `enabled: true`, spawn `node <scripts>/caduceus.mjs accounts sync --json`,
   then `writeState()`. Key is visible while typing — the dialog title says so.
   `vitriol caduceus signup --only <provider>` remains the hidden path.
2. **Wizard / OAuth** (TUI suspend): disable raw mode + mouse capture, leave
   the alternate screen, spawn the CLI with inherited stdio, wait, restore,
   reload. Non-TTY fallback prints the command instead of spawning.

## File map

| File | Change |
|---|---|
| `officina/.pi/extensions/_shared/caduceus-prefs.ts` | new: types, validated reader, pure `applyPrefs` (filter/order) + `capCandidates` (caps), `allowPaidOwners`, env seam |
| `officina/.pi/extensions/_shared/caduceus-prefs.test.ts` | new: parity, deny, pin, rank, free_only, allow_paid, caps, corrupt |
| `officina/.pi/extensions/caduceus/catalog.ts` | `toolCandidates(raw, {freeOnly, allowPaidOwners})`; `freeToolCandidates` kept as wrapper |
| `officina/.pi/extensions/caduceus/catalog.test.ts` | paid/allow-paid cases |
| `officina/.pi/extensions/caduceus/state.ts` | v2 fields + builder |
| `officina/.pi/extensions/caduceus/state.test.ts` | v2 builder + v1 compat |
| `officina/.pi/extensions/caduceus/index.ts` | provider catalog fetch/cache, v2 write, `/caduceus-add` |
| `officina/.pi/extensions/resilient-default/index.ts` | prefs-aware pool, caps, rebuild signature |
| `officina/.pi/extensions/adaptive-router/index.ts` | same |
| `officina/.pi/extensions/auto-route/index.ts` | freeOnly from prefs |
| `officina/tui/src/tui/caduceus.rs` | page state, prefs model, tests |
| `officina/tui/src/tui/state.rs` | page state fields, open/close, prefs save |
| `officina/tui/src/tui/layout.rs` | full-bleed tabbed renderer |
| `officina/tui/src/tui/mod.rs` | keymap, action relay, suspend/resume |
| `officina/docs/officina-guide.md` | page section |

## Stages

1. **Prefs + enforcement (TS)** — `caduceus-prefs.ts`, `toolCandidates`,
   router wiring, tests. Gate: `npm test`, `npm run typecheck`.
2. **Facts v2** — state builder additions, provider catalog fetch/cache,
   `/caduceus-add`, tests. Gate: `npm test`, `npm run typecheck`.
3. **TUI page** — tabs, prefs load/save, keymap, render, tests. Gate:
   `cargo test`, `cargo fmt --check`, `cargo build --release` + install.
4. **Onboarding + docs** — suspend/resume, wizard/OAuth handoff, guide
   section. Gate: `cargo test`, `cargo fmt --check`, rebuild/install.

## Invariants

- **I1 parity** — no prefs file (or all-neutral prefs) ⇒ candidate order and
  selection byte-identical to today. Pinned by `caduceus-prefs.test.ts`
  golden matrix and existing router tests.
- **I2 filter/reorder only** — prefs never invent candidates, never route an
  owner with no key, never bypass the catalog.
- **I3 free derived** — free flags are facts; only policy toggles are
  writable.
- **I4 Rust does no HTTP and never sees secrets** — TUI writes only
  `caduceus-prefs.json`; key entry and sync run in the extension/CLI.
- **I5 state backward compatible** — v1 `caduceus-state.json` still renders
  (missing `providers`/`candidates` degrade to the old summary), pinned by a
  serde-default test.
- **I6 atomic + corrupt-safe** — both files written tmp+rename; invalid
  prefs = ignored + surfaced, never a crash or a silent policy change.
- **I7 routing only** — `accounts.json` and gateway state are untouched by
  every toggle except the explicit add-key/OAuth flows.
- **I8 no mid-turn switch** — prefs are read at candidate-pool rebuild;
  prefs mtime joins the health signature. An in-flight turn never changes
  model because of a toggle.

## Risks

- Provider catalog requires the vendor CLI; cache 10 min, fall back to
  accounts + OAuth list, page shows "catalog unavailable" instead of lying.
- `allow_paid` widens the pool for the first time — explicit per-provider
  opt-in, tests pin that default behaviour is unchanged.
- Terminal suspend must restore raw mode, mouse capture, and the alternate
  screen, then reload facts; a failed resume is worse than no suspend, so
  non-TTY refuses.
- Inline key input echoes (pi has no password mode); the dialog says so and
  the wizard is the hidden alternative.

## Out of scope

- Mutation of `accounts.json` enablement from the TUI (still
  `vitriol caduceus accounts … --sync`).
- Editing gateway pricing/free classification.
- Per-model `allow_paid` (owner-level only in v1).
- Sidebar changes beyond the existing caduceus section.
